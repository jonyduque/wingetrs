//! Protocol and loop of the elevated bridge helper (`wingetrs bridge-worker`): one JSON object
//! per line each way. The helper only ever runs `bridge::command` of a winget job with a valid
//! id, or `bridge::winget_source_update`; anything else closes the channel.
use super::{BridgeExec, BridgeJob, Tool};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::path::Path;

/// Longest line either side reads or writes, without its `\n`.
pub const LINE_CAP: usize = 64 * 1024;

/// What the TUI asks the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// `winget upgrade --id <target> ...` (the fixed command of `bridge::command`).
    Upgrade { target: String },
    /// `winget source update --name winget ...`.
    // Struct variants, not unit ones: serde lets extra fields through on a unit variant of an
    // internally tagged enum even with `deny_unknown_fields`.
    SourceUpdate {},
    /// Ends the helper.
    Quit {},
}

/// What the helper answers: any number of `Line`, then one `End` or `Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Line {
        text: String,
    },
    End {
        code: i32,
    },
    /// Nothing ran (refused request), or the tool could not be started.
    Error {
        message: String,
        not_found: bool,
    },
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

/// `msg` as one line (with its `\n`); refused when it would pass `LINE_CAP`.
pub fn encode<T: Serialize>(msg: &T) -> std::io::Result<String> {
    let mut text = serde_json::to_string(msg).map_err(|e| invalid(e.to_string()))?;
    if text.len() > LINE_CAP {
        return Err(invalid(format!("mensagem com {} bytes", text.len())));
    }
    text.push('\n');
    Ok(text)
}

/// One line (without its `\n`) as a message; unknown kinds and extra fields are refused.
pub fn decode<T: DeserializeOwned>(line: &str) -> std::io::Result<T> {
    serde_json::from_str(line).map_err(|e| invalid(e.to_string()))
}

/// The next line, without `\n`; `None` at the end of the stream. A line longer than
/// `LINE_CAP` is refused before it is buffered whole.
pub fn read_line(reader: &mut dyn BufRead) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    reader
        .take(LINE_CAP as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.len() > LINE_CAP {
        return Err(invalid(format!("linha com mais de {LINE_CAP} bytes")));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| invalid("linha que não é UTF-8"))
}

fn send(writer: &mut dyn Write, reply: &Reply) -> std::io::Result<()> {
    writer.write_all(encode(reply)?.as_bytes())?;
    writer.flush()
}

/// The helper's loop: reads requests from `reader`, runs each with `exec` and writes the
/// output lines and the end to `writer`. Returns at the end of `reader` or at `quit`; an
/// invalid line (too long, not a known request) ends it with an error, without running it.
pub fn serve(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    exec: BridgeExec<'_>,
) -> std::io::Result<()> {
    while let Some(line) = read_line(reader)? {
        let request: Request = decode(&line).inspect_err(|e| {
            tracing::warn!(error = %e, "bridge worker: invalid request; closing");
        })?;
        let (program, args) = match request {
            Request::Quit {} => return Ok(()),
            Request::SourceUpdate {} => super::winget_source_update(),
            Request::Upgrade { target } => {
                if !super::valid_target(Tool::Winget, &target) {
                    tracing::warn!(len = target.len(), "bridge worker: invalid target refused");
                    send(
                        writer,
                        &Reply::Error {
                            message: "identificador inválido".to_string(),
                            not_found: false,
                        },
                    )?;
                    continue;
                }
                let job = BridgeJob {
                    tool: Tool::Winget,
                    name: target.clone(),
                    target,
                    installed: None,
                    available: None,
                    scope: None,
                };
                super::command(&job, Path::new(""), "")
            }
        };
        tracing::info!(
            cmd = %super::command_text(&program, &args),
            "bridge worker: running"
        );
        // A broken channel does not stop the tool; the first write error ends the loop after it.
        let mut broken: Option<std::io::Error> = None;
        let result = exec(&program, &args, &mut |text: &str| {
            if broken.is_none()
                && let Err(e) = send(
                    writer,
                    &Reply::Line {
                        text: text.to_string(),
                    },
                )
            {
                broken = Some(e);
            }
        });
        if let Some(e) = broken {
            return Err(e);
        }
        let reply = match result {
            Ok((code, _)) => Reply::End { code },
            Err(e) => Reply::Error {
                message: e.to_string(),
                not_found: e.kind() == std::io::ErrorKind::NotFound,
            },
        };
        send(writer, &reply)?;
    }
    Ok(())
}
