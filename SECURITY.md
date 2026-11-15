# Security policy

ratclone runs rclone with your cloud credentials and can copy, move and
delete files on your accounts, so security reports are taken seriously.

## Reporting a vulnerability

Please report vulnerabilities privately through
[GitHub's security advisories](https://github.com/SyscallBrain/ratclone/security/advisories/new),
not in a public issue. Include the version (`ratclone --version`), the output
of `rclone version`, the remote type involved and the steps to reproduce.

You can expect a first answer within a week. Once a fix is released, the
advisory is published with credit to you, unless you prefer otherwise.

## Scope

ratclone is a front end for rclone: it starts its own `rclone rcd` on a
private unix socket (`$XDG_RUNTIME_DIR/ratclone/rc.sock`) and talks to it
through the rc API. Problems in rclone itself or in a cloud provider belong to
[rclone's security policy](https://github.com/rclone/rclone/security/policy)
or to the provider concerned.

## Supported versions

Only the latest release receives security fixes.
