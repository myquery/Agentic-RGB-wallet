# Wallet and recipient user services

The wallet API serves the built UI directly. Run Alice, Bob, and Carol as user
services on ports 3030, 3031, and 3032; do not run Vite for normal use.

Install the template from the repository root:

```bash
mkdir -p ~/.config/systemd/user
cp deploy/systemd/rgb402-wallet@.service \
   deploy/systemd/rgb402-recipient.service \
   deploy/systemd/rgb402-ngrok.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now rgb402-wallet@alice rgb402-wallet@bob rgb402-wallet@carol
systemctl --user enable --now rgb402-recipient rgb402-ngrok
loginctl enable-linger "$USER"
```

Lingering starts these user services after a reboot without an interactive login.
The regtest nodes must also be running; systemd retries a wallet API until its
node is available. The recipient and ngrok services require the existing ngrok
configuration. The tunnel writes its active public hostname to `.var/regtest/recipient-domain` and
restarts the services that embed that hostname when it changes. To request a
reserved hostname owned by the configured ngrok account, put only that hostname
in `.var/regtest/ngrok-domain`. Together the services keep WebFinger and Carol's
public catalog reachable through port 3050. Merchant order journals are scoped to
the public hostname because merchant identity includes that hostname. A hostname
change preserves older journals and copies only store settings/products into a
fresh journal namespace.

The journal lock is advisory and held by an open file descriptor. A second
wallet process using the same journal is refused, while a crash or reboot
releases the lock automatically. Reservations remain durable and are recovered
as uncertain, never retried automatically.

```bash
systemctl --user status rgb402-wallet@alice rgb402-wallet@bob rgb402-wallet@carol \
  rgb402-recipient rgb402-ngrok
journalctl --user -u rgb402-wallet@alice -f
```
