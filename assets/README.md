# Icon assets

Place the two state icons here:

- `taskbar-enable-autohide.ico`
- `taskbar-disable-autohide.ico`

`build.rs` embeds one of these icons into the executable (with fallback to
`taskbar-autohide.ico` for backward compatibility). At runtime, the app picks
the icon matching the current possible action:

- enable icon when auto-hide is currently disabled
- disable icon when auto-hide is currently enabled
