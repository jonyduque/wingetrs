Status: DONE

Commits:
88f0b4c feat(cli): yes/no prompt as buttons

Gate:
[OK]   clippy 143s
[OK]   test (1961 passed, 87 ignored) 127s
[OK]   gate verde

Mutation proofs:
- Initial focus on Sim: `mutate.ps1 -Path src/cli/picker.rs -Anchor 'let mut yes = false;' -Replacement 'let mut yes = true;' -TestCommand 'cargo test --quiet --test it selector::yes_no'`
  [OK] mutação PEGA: 'let mut yes = false;' -> 'let mut yes = true;' fez o teste falhar (exit 101).  (yes_no_enter_keeps_no, yes_no_arrow_then_enter_is_yes)
- Esc => Sim: anchor 'KeyCode::Esc => break false,' -> 'KeyCode::Esc => break true,'
  [OK] mutação PEGA: 'KeyCode::Esc => break false,' -> 'KeyCode::Esc => break true,' fez o teste falhar (exit 101).

Screen of test 1 (fake::plain, first draw):
Continuar?  [ Sim ]  [ Não ]\r\n
←/→ escolher · Enter confirmar · s/n · Esc = Não\r\n
(then erased; ends with "Continuar? Não\r\n")

Decisions:
- Focus style uses theme.accent()+REVERSED and theme.muted() (same ratatui/theme path the picker already uses), not style.rs/anstyle; style.rs untouched.
- yes_no has its own small loop and draw_buttons (two styles in one line); Session reused for raw mode/erase. Removed the now-unused yes_no_items.
- A long question is cut so the buttons keep their room.
- TDD red step not run separately for the new tests (the old test was replaced); the mutations show the tests fail for the right reason.
- wi_cli.rs only got an extra assertion for the button line (existing ones still hold).

Concerns: none. (Mid-run, wi_cli bridge_*_progress tests failed from Task 215's in-progress work; they passed in the final gate.)
