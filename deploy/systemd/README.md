# Wallet API user services

The wallet API serves the built UI directly. Run Alice, Bob, and Carol as user
services on ports 3030, 3031, and 3032; do not run Vite for normal use.

Install the template from the repository root:

```bash
mkdir -p ~/.config/systemd/user
cp deploy/systemd/rgb402-wallet@.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now rgb402-wallet@alice rgb402-wallet@bob rgb402-wallet@carol
loginctl enable-linger "$USER"
```

Lingering starts these user services after a reboot without an interactive login.
The regtest nodes must also be running; systemd retries a wallet API until its
node is available.

The journal lock is advisory and held by an open file descriptor. A second
wallet process using the same journal is refused, while a crash or reboot
releases the lock automatically. Reservations remain durable and are recovered
as uncertain, never retried automatically.

```bash
systemctl --user status rgb402-wallet@alice rgb402-wallet@bob rgb402-wallet@carol
journalctl --user -u rgb402-wallet@alice -f
```
