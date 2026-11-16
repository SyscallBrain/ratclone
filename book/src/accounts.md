# Accounts

ratclone uses the accounts (rclone *remotes*) in your rclone configuration,
`rclone.conf`. Accounts added or edited from the app are written there, so
`rclone` on the command line sees them too.

These keys work in the accounts list (`Tab` to focus it):

| Key | Action |
|-----|--------|
| `a` | add an account |
| `e` | edit the selected account |
| `A` | sign in again (a new token) |
| `dd` or `D` | remove the account |
| `r` or `R` | refresh its usage |

## Adding an account

`a` asks for a type: type to filter the list, then `Enter`. Give the account a
name, and fill in its fields:

- `Tab` shows the advanced fields,
- `F1` shows the full help of a field,
- `Ctrl-d` puts a field back to rclone's default,
- `Ctrl-o` on a remote path picks one of your accounts or the local disk
  (for crypt, alias and union accounts).

`Enter` on the last field, or `Ctrl-s`, sends it. Then answer rclone's own
questions, such as the login.

## Editing

`e` opens the form filled in. Passwords show as `••••• (unchanged)` and are
only sent if you type a new one. Only the fields you change are sent.

## Browser logins

Google Drive, Dropbox, OneDrive and other accounts log in through a browser.

- **On your own computer with rclone 1.75 or newer**, the browser opens and
  the app waits for you. Older versions cannot cancel this login reliably,
  so the app automatically offers the authorization-token flow below. You
  can run the displayed command in another terminal on the same computer.
- **Over SSH** there is no browser, so the app shows the command to run on a
  machine that has one, such as `rclone authorize "dropbox"`. `y` copies it
  (through OSC 52, so it works over SSH too), and you paste the result back.

`L` switches between the two ways if the guess was wrong, and `Esc` cancels. A
cancelled new account is removed, so nothing half-made stays in
`rclone.conf`. `A` goes through the login again when a token has expired.

## Removing an account

`dd` or `D` removes the account from rclone's configuration, after asking. The
files in the account are not touched. Accounts built on it (crypt, alias,
union) are listed in the question. Removing an account is refused while
transfers use it.
