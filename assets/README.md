# Icon assets

Place the two state icons here:

- `taskbar-autohide-disabled.ico`
- `taskbar-autohide-enabled.ico`

`build.rs` embeds the disabled icon into the executable (with fallback to
`taskbar-autohide.ico` for backward compatibility).
