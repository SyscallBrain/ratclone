# Command-line options

```text
ratclone [OPTIONS]
```

| Option | Description |
|---|---|
| `--demo` | Run with made-up accounts and files; nothing touches rclone. |
| `--rclone PATH` | The rclone binary to run. Default: `rclone_path` in the [configuration](configuration.md), or `rclone` on your `PATH`. |
| `--socket PATH` | The unix socket for ratclone's own `rclone rcd`. Default: `$XDG_RUNTIME_DIR/ratclone/rc.sock`, or `/tmp/ratclone-UID/rc.sock` without `XDG_RUNTIME_DIR`. |
| `-h`, `--help` | Print the options. |
| `-V`, `--version` | Print the version. |
