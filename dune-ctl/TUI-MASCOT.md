# TUI Mascot Direction

Design note for the `dune-ctl` Ratatui mascot: Maud'Dib, a terminal rat, on a
dune or riding a sandworm.

## Default: Portable Text Animation

The first pass is a pure Ratatui/crossterm text animation. Do not add terminal
image dependencies for the default mode.

Reasons:

- Works over SSH.
- Works in tmux and ordinary terminals.
- No Kitty/Sixel/iTerm2 capability detection.
- No binary asset loading.
- Fits the existing `dune-ctl` event loop, which already redraws regularly.

Implemented first pass:

- Small right-aligned mascot in the header on terminals at least 96 columns
  wide.
- Six ASCII frames, derived from app uptime.
- Hidden automatically on narrow terminals so operational status keeps priority.

Good future text improvements:

- Small corner mascot in the header or log pane.
- Two-to-six frame Unicode/ASCII animation.
- Sandworm body made from block characters or shaded spans.
- Dune wave made from `~`, `_`, `-`, and block/half-block characters.
- Rat silhouette dancing, peeking over a dune, or surfing/riding the worm.

Keep it optional and unobtrusive. It should not resize tables, obscure map
state, or make the TUI noisy during operational work.

## Suggested Text Frames

Example style only; tune after seeing it in the live TUI:

```text
  /\_/\
 ( o.o )  ~~~~
  > ^ <  _/\/\_
```

```text
  /\_/\
 ( -.- )  __/\/
  / ^ \ ~~~~
```

For a worm-riding frame, keep the rat tiny and make the worm the motion:

```text
  /\_/\
 ( o.o )__
__/^^^^\___
```

```text
    /\_/\
 __( o.o )
___/^^^^\__
```

## Integration Notes

The current TUI lives under:

```text
dune-ctl/ctl/src/tui/
```

Relevant files:

- `app.rs`: event loop, timing, and application state.
- `ui.rs`: layout and drawing.
- `mod.rs`: terminal setup/teardown.

Current implementation:

- `App` has a `started_at: Instant` field.
- Derive frame index from elapsed time, not from input events.
- Render the mascot inside a fixed-size `Rect` so the layout never shifts.
- Hide it when the terminal is too narrow.

Avoid:

- Unicode that renders at ambiguous widths in common terminals.
- Rapid frame rates; 4-6 FPS is enough.
- Full-screen animation.

## Future Enhanced Mode: Bitmap Images

If we later want real pixel art, add it as an optional feature after the text
mascot is stable.

Candidate approach:

- Add optional `ratatui-image`.
- Detect terminal image support.
- Prefer Kitty protocol when available.
- Consider Sixel or iTerm2 protocol if the library supports them cleanly.
- Fall back to the text mascot automatically.

Potential feature shape:

```text
default: text mascot
feature "tui-images": enable ratatui-image support
runtime: choose Kitty/Sixel/iTerm2 only when supported
fallback: text mascot
```

Do not make image protocol support required for normal `dune-ctl` use. This is
an SSH operations tool first.
