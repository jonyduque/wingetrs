"""Regenerate test fixtures from real downloads.

Usage: python -I scripts/make_fixtures.py <source2.msix extracted dir> <scoop Main.tar.gz> <versionData.mszyml> <merged manifest>

Produces (all under tests/fixtures/):
  winget/index-mini.db              reduced v2 index: KEEP packages + all their mapping rows
  winget/source2-mini.msix          ZIP with Public/index.db + AppxBlockMap.xml (real format, recomputed hashes)
  winget/source2-mini-corrupt.msix  same, one byte of index.db flipped (hashes no longer match)
  winget/powertoys.versionData.mszyml / .yaml (decoded oracle)
  winget/powertoys-0.101.2362.0.merged.yaml
  scoop/Main-mini.tar.gz            codeload layout (Main-master/bucket/*.json), KEEP_SCOOP manifests + one invalid

Not produced by this script (needs 7-Zip, kept out of the gate): zip/deflate64.zip, made by hand from
a.txt (22 B line x20) and sub/b.txt (22 B line x20) with
  7z a -tzip -mm=Deflate64 tests/fixtures/zip/deflate64.zip <tmp>/*
(check with `7z l -slt`: Method = Deflate64). See tests/install_extract.rs.
"""
import base64, hashlib, io, os, shutil, sqlite3, struct, sys, tarfile, zipfile, zlib

ROOT = os.path.join(os.path.dirname(__file__), '..', 'tests', 'fixtures')
KEEP = ['Microsoft.PowerToys', 'Mozilla.Firefox', '7zip.7zip', 'Google.Chrome', 'Git.Git',
        'Microsoft.VisualStudioCode', 'VideoLAN.VLC', 'Notepad++.Notepad++', 'Python.Python.3.13',
        'Microsoft.WindowsTerminal', 'OBSProject.OBSStudio', 'Discord.Discord', 'Valve.Steam',
        'Spotify.Spotify', 'GitHub.cli', 'BurntSushi.ripgrep.MSVC', 'JohnMacFarlane.Pandoc',
        'TheDocumentFoundation.LibreOffice', 'Microsoft.PowerShell', 'Docker.DockerDesktop']
KEEP_SCOOP = ['7zip', 'git', 'ripgrep', 'gh', 'ffmpeg', 'aria2', 'nodejs-lts', 'python']
MAP_TABLES = ['pfns2', 'productcodes2', 'norm_names2', 'norm_publishers2', 'upgradecodes2']
BLOCK = 65536


def mszip_decode(b):
    total = struct.unpack('<Q', b[8:16])[0]
    pos, out, prev = 24, b'', b''
    while pos < len(b) and len(out) < total:
        size = struct.unpack('<I', b[pos:pos + 4])[0]
        blk = b[pos + 4:pos + 4 + size]
        assert blk[:2] == b'CK'
        d = zlib.decompressobj(-15, zdict=prev) if prev else zlib.decompressobj(-15)
        chunk = d.decompress(blk[2:])
        out += chunk
        prev = chunk[-32768:]
        pos += 4 + size
    return out


def mini_index(src_db, dst_db):
    if os.path.exists(dst_db):
        os.remove(dst_db)
    src = sqlite3.connect(src_db)
    dst = sqlite3.connect(dst_db)
    for (sql,) in src.execute("select sql from sqlite_master where type='table'"):
        dst.execute(sql)
    for (sql,) in src.execute("select sql from sqlite_master where type='index' and sql is not null"):
        dst.execute(sql)
    dst.executemany('insert into metadata values (?,?)', src.execute('select * from metadata'))
    q = ','.join('?' * len(KEEP))
    rows = src.execute(f'select * from packages where id in ({q})', KEEP).fetchall()
    assert len(rows) == len(KEEP), f'missing packages: {set(KEEP) - {r[1] for r in rows}}'
    dst.executemany('insert into packages values (?,?,?,?,?,?,?,?)', rows)
    ids = [r[0] for r in rows]
    qi = ','.join('?' * len(ids))
    for t in MAP_TABLES:
        dst.executemany(f'insert into {t} values (?,?)', src.execute(f'select * from {t} where package in ({qi})', ids))
    for t, m, col in [('tags2', 'tags2_map', 'tag'), ('commands2', 'commands2_map', 'command')]:
        maps = src.execute(f'select * from {m} where package in ({qi})', ids).fetchall()
        dst.executemany(f'insert into {m} values (?,?)', maps)
        keys = sorted({r[0] for r in maps})
        if keys:
            qk = ','.join('?' * len(keys))
            dst.executemany(f'insert into {t} values (?,?)', src.execute(f'select * from {t} where rowid in ({qk})', keys))
    dst.commit()
    dst.execute('vacuum')
    dst.close()
    n = sqlite3.connect(dst_db).execute('select count(*) from packages').fetchone()[0]
    assert n == len(KEEP), n


def blockmap_xml(name, data):
    blocks = []
    for i in range(0, len(data), BLOCK):
        h = base64.b64encode(hashlib.sha256(data[i:i + BLOCK]).digest()).decode()
        blocks.append(f'<Block Hash="{h}"/>')
    fh = base64.b64encode(hashlib.sha256(data).digest()).decode()
    return ('<?xml version="1.0" encoding="UTF-8" standalone="no"?>'
            '<BlockMap xmlns="http://schemas.microsoft.com/appx/2010/blockmap" '
            'xmlns:b4="http://schemas.microsoft.com/appx/2021/blockmap" IgnorableNamespaces="b4" '
            'HashMethod="http://www.w3.org/2001/04/xmlenc#sha256">'
            f'<File Name="{name}" Size="{len(data)}" LfhSize="45"><b4:FileHash Hash="{fh}"/>'
            + ''.join(blocks) + '</File></BlockMap>')


def write_msix(path, db_bytes, map_bytes):
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        z.writestr('Public/index.db', db_bytes)
        z.writestr('AppxBlockMap.xml', map_bytes)
        z.writestr('[Content_Types].xml', '<?xml version="1.0"?><Types/>')


def main():
    idx_dir, scoop_tgz, vd, manifest = sys.argv[1:5]
    os.makedirs(os.path.join(ROOT, 'winget'), exist_ok=True)
    os.makedirs(os.path.join(ROOT, 'scoop'), exist_ok=True)

    db = os.path.join(ROOT, 'winget', 'index-mini.db')
    mini_index(os.path.join(idx_dir, 'Public', 'index.db'), db)
    data = open(db, 'rb').read()
    bm = blockmap_xml('Public\\index.db', data).encode()
    write_msix(os.path.join(ROOT, 'winget', 'source2-mini.msix'), data, bm)
    bad = bytearray(data)
    bad[len(bad) // 2] ^= 0xFF
    write_msix(os.path.join(ROOT, 'winget', 'source2-mini-corrupt.msix'), bytes(bad), bm)

    shutil.copy(vd, os.path.join(ROOT, 'winget', 'powertoys.versionData.mszyml'))
    open(os.path.join(ROOT, 'winget', 'powertoys.versionData.yaml'), 'wb').write(mszip_decode(open(vd, 'rb').read()))
    shutil.copy(manifest, os.path.join(ROOT, 'winget', 'powertoys-0.101.2362.0.merged.yaml'))

    out = os.path.join(ROOT, 'scoop', 'Main-mini.tar.gz')
    with tarfile.open(scoop_tgz) as src, tarfile.open(out, 'w:gz') as dst:
        kept = 0
        for m in src.getmembers():
            base = os.path.basename(m.name)
            if m.isdir() and m.name.rstrip('/') in ('Main-master', 'Main-master/bucket'):
                dst.addfile(m)
            elif m.name.startswith('Main-master/bucket/') and base[:-5] in KEEP_SCOOP and base.endswith('.json'):
                dst.addfile(m, src.extractfile(m))
                kept += 1
            elif m.name == 'Main-master/README.md':
                dst.addfile(m, src.extractfile(m))
        assert kept == len(KEEP_SCOOP), kept
        bad = b'{ "version": "1.0", "url": '  # truncated JSON on purpose
        ti = tarfile.TarInfo('Main-master/bucket/broken.json')
        ti.size = len(bad)
        dst.addfile(ti, io.BytesIO(bad))
    print('fixtures ok')


if __name__ == '__main__':
    main()
