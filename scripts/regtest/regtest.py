#!/usr/bin/env python3
"""Single-user local regtest lifecycle and wallet acceptance. No third-party Python packages."""
import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = ROOT / '.dev/rgb-lightning-node'
STATE = ROOT / '.var/regtest'
LOGS = STATE / 'logs'
PIN = 'e4008278c80495ea8a8514580b899e771feff872'
LDK_PIN = 'e2a0b8e24dc919ccef289f103fd1f8e1974fcc1d'
REPOSITORY = 'https://github.com/RGB-Tools/rgb-lightning-node.git'
PROJECT = 'rgb402-regtest'
RUNTIME = 'rgb402-rln-runtime:e4008278'
DEBIAN = 'debian:trixie-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132'
IMAGES = {
    'bitcoind': 'registry.gitlab.com/hashbeam/docker/bitcoind:30.2@sha256:5473c951ca8a703f7c15d9a353f99135e0a1f0cc7ef976f29d073160950098bf',
    'electrs': 'registry.gitlab.com/hashbeam/docker/electrs:0.11.0@sha256:560778444e7fa47a718ffaee371de9066e61aab4a674aae9ab90038899c50da6',
    'proxy': 'ghcr.io/rgb-tools/rgb-proxy-server:0.3.0@sha256:b06fe0f234c53030d54fe1e704daf4bc6ff2879a335124353175130a5112d313',
}
PORTS = {'alice': 3101, 'bob': 3102}
# Deliberately public, development-only password. Never use these wallets outside regtest.
PASSWORD = 'rgb402-local-regtest-only'


class Failure(RuntimeError):
    pass


class ApiError(Failure):
    def __init__(self, node, endpoint, status, name):
        self.name = name
        super().__init__(f'{node} /{endpoint}: HTTP {status} ({name})')


def say(message):
    print(message, flush=True)


def safe_paths():
    # All lifecycle paths are fixed; reject symlinked parents before any mutation.
    for path in (ROOT / '.dev', UPSTREAM, ROOT / '.var', STATE, STATE / 'data'):
        if path.is_symlink():
            raise Failure(f'Refusing symlinked development path: {path}')


def run(args, *, log=None, env=None):
    args = [str(a) for a in args]
    if log:
        LOGS.mkdir(parents=True, exist_ok=True)
        with (LOGS / log).open('a') as stream:
            result = subprocess.run(args, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
        if result.returncode:
            raise Failure(f'{args[0]} failed; see {LOGS / log}')
        return ''
    result = subprocess.run(args, cwd=ROOT, env=env, text=True, capture_output=True)
    if result.returncode:
        raise Failure(f'{args[0]} failed: {result.stderr.strip() or result.stdout.strip()}')
    return result.stdout.strip()


def compose(*args):
    return run(['docker', 'compose', '-p', PROJECT, '-f', STATE / 'compose.yaml', *args])


def btc(*args):
    value = compose('exec', '-T', '-u', 'blits', 'bitcoind', 'bitcoin-cli',
                    '-regtest', '-rpcwallet=miner', *map(str, args))
    try:
        return json.loads(value)
    except ValueError:
        return value


def save(name, data):
    path = STATE / name
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(data, indent=2) + '\n')
    temporary.replace(path)


def load(name):
    return json.loads((STATE / name).read_text())


def api(node, endpoint, body=None, timeout=120):
    request = urllib.request.Request(
        f'http://127.0.0.1:{PORTS[node]}/{endpoint}',
        data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            result = json.load(response)
        # Never return, save or print the initialization mnemonic.
        return {} if endpoint == 'init' else result
    except urllib.error.HTTPError as error:
        try:
            name = json.loads(error.read()).get('name', 'UnknownError')
        except (ValueError, UnicodeError):
            name = 'InvalidResponse'
        raise ApiError(node, endpoint, error.code, name) from None
    except (OSError, urllib.error.URLError) as error:
        raise Failure(f'{node} /{endpoint} unavailable or timed out; inspect node logs') from error


def wait_for(label, check, timeout=120):
    deadline = time.monotonic() + timeout
    last = ''
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (Failure, OSError, ValueError) as error:
            last = str(error)
        time.sleep(1)  # bounded polling interval, never a guessed readiness delay
    raise Failure(f'{label} failed to become ready within {timeout}s. {last}')


def index_height():
    with socket.create_connection(('127.0.0.1', 55001), timeout=3) as connection:
        connection.sendall(b'{"id":1,"method":"blockchain.headers.subscribe","params":[]}\n')
        result = json.loads(connection.makefile().readline())
        return result['result']['height']


def synced():
    height = btc('getblockcount')
    wait_for('Electrs chain synchronization', lambda: index_height() >= height)
    for node in PORTS:
        wait_for(f'{node} chain synchronization', lambda node=node: api(node, 'networkinfo', timeout=5)['height'] >= height)


def mine(blocks):
    btc('-generate', blocks)
    say(f'  Mined {blocks} regtest blocks')


def docker_ready():
    if not shutil.which('docker'):
        raise Failure('Docker unavailable: install Docker Engine and Compose >= 2.24.4')
    try:
        run(['docker', 'info', '--format', '{{.ServerVersion}}'])
        version = run(['docker', 'compose', 'version', '--short']).lstrip('v').split('-')[0]
        if tuple(map(int, version.split('.')[:3])) < (2, 24, 4):
            raise Failure('Compose >= 2.24.4 required for !override')
    except Failure as error:
        raise Failure(f'Docker unavailable, permission denied, or unsupported Compose: {error}') from error


def checkout_and_build():
    for name in ('git', 'cargo', 'cc', 'cmake'):
        if not shutil.which(name):
            raise Failure(f'Missing prerequisite: {name}')
    if not UPSTREAM.exists():
        UPSTREAM.parent.mkdir(parents=True, exist_ok=True)
        run(['git', 'clone', '--no-checkout', REPOSITORY, UPSTREAM], log='checkout.log')
        run(['git', '-C', UPSTREAM, 'checkout', '--detach', PIN], log='checkout.log')
    if run(['git', '-C', UPSTREAM, 'rev-parse', 'HEAD']) != PIN:
        raise Failure(f'Upstream revision mismatch; expected {PIN}. Refusing to change it automatically.')
    if run(['git', '-C', UPSTREAM, 'status', '--porcelain', '--untracked-files=no']):
        raise Failure('Pinned upstream checkout is modified; restore it before building')
    run(['git', '-C', UPSTREAM, 'submodule', 'update', '--init', '--depth', '1'], log='checkout.log')
    if run(['git', '-C', UPSTREAM / 'rust-lightning', 'rev-parse', 'HEAD']) != LDK_PIN:
        raise Failure('rust-lightning submodule revision mismatch')
    env = os.environ.copy()
    env.update(CARGO_NET_GIT_FETCH_WITH_CLI='true', CARGO_BUILD_JOBS='2',
               CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_DEV_INCREMENTAL='false')
    say('Building pinned node (first build downloads dependencies; see logs/build.log)')
    run(['cargo', 'build', '--locked', '--manifest-path', UPSTREAM / 'Cargo.toml'], log='build.log', env=env)
    run(['docker', 'build', '-t', RUNTIME, '-f', ROOT / 'scripts/regtest/Dockerfile.runtime',
         ROOT / 'scripts/regtest'], log='runtime-build.log')
    save('build.json', {'node_commit': PIN, 'rust_lightning_commit': LDK_PIN,
                        'rustc': run(['rustc', '--version']), 'images': IMAGES})


def write_compose():
    uid, gid = os.getuid(), os.getgid()
    for name in ('alice', 'bob', 'core', 'index'):
        (STATE / 'data' / name).mkdir(parents=True, exist_ok=True)
    # Extend the pinned upstream services, overriding only isolation and image digests.
    lines = ['services:']
    for service, host, internal, folder, destination in (
        ('bitcoind', 28443, 18443, 'core', '/srv/app/.bitcoin'),
        ('electrs', 55001, 50001, 'index', '/srv/app/db'),
        ('proxy', 3300, 3000, None, None),
    ):
        lines += [f'  {service}:', '    extends:', f'      file: {json.dumps(str(UPSTREAM / "compose.yaml"))}',
                  f'      service: {service}', f'    image: {IMAGES[service]}',
                  f'    ports: !override ["127.0.0.1:{host}:{internal}"]']
        if folder:
            lines += [f'    environment: {{MYUID: {uid}, MYGID: {gid}}}',
                      f'    volumes: !override [{json.dumps(str(STATE / "data" / folder) + ":" + destination)}]']
    for node, peer in (('alice', 19735), ('bob', 19736)):
        lines += [f'  {node}:', f'    image: {RUNTIME}', f'    user: "{uid}:{gid}"',
                  '    command: ["/data", "--daemon-listening-port", "3001", "--ldk-peer-listening-port", "9735", "--network", "regtest", "--disable-authentication"]',
                  f'    ports: ["127.0.0.1:{PORTS[node]}:3001", "127.0.0.1:{peer}:9735"]',
                  '    volumes:',
                  f'      - {json.dumps(str(UPSTREAM / "target/debug/rgb-lightning-node") + ":/usr/local/bin/rgb-lightning-node:ro")}',
                  f'      - {json.dumps(str(STATE / "data" / node) + ":/data")}',
                  '    stop_grace_period: 60s']
    (STATE / 'compose.yaml').write_text('\n'.join(lines) + '\n')


def api_ready(node):
    try:
        api(node, 'nodeinfo', timeout=3)
        return True
    except ApiError as error:
        return error.name == 'LockedNode'


def start():
    docker_ready()
    checkout_and_build()
    write_compose()
    # Compose reports any port collision without stopping unrelated containers.
    compose('up', '-d', 'bitcoind')
    wait_for('bitcoind RPC on localhost:28443', lambda: btc('getblockchaininfo'))
    wallets = btc('listwallets')
    if 'miner' not in wallets:
        known = [wallet['name'] for wallet in btc('listwalletdir')['wallets']]
        btc('loadwallet' if 'miner' in known else 'createwallet', 'miner')
    height = btc('getblockcount')
    if height < 103:
        mine(103 - height)
    compose('up', '-d', 'electrs', 'proxy')
    wait_for('Electrs on localhost:55001', lambda: index_height() >= btc('getblockcount'))
    compose('up', '-d', 'alice', 'bob')
    for node in PORTS:
        wait_for(f'{node} API on localhost:{PORTS[node]}', lambda node=node: api_ready(node))
    say('Infrastructure and both node APIs ready (nodes may still need bootstrap/unlock).')


def unlock(node):
    try:
        api(node, 'nodeinfo', timeout=5)
        return
    except ApiError as error:
        if error.name != 'LockedNode':
            raise
    try:
        api(node, 'init', {'password': PASSWORD})
    except ApiError as error:
        if error.name != 'AlreadyInitialized':
            raise
    api(node, 'unlock', {'password': PASSWORD,
                         'ldk_chain_sync': {'mode': 'BlockSync', 'config': {
                             'bitcoind_rpc_username': 'user', 'bitcoind_rpc_password': 'password',
                             'bitcoind_rpc_host': 'bitcoind', 'bitcoind_rpc_port': 18443}},
                         'indexer_url': 'electrs:50001', 'announce_addresses': [], 'announce_alias': node})


def channels(node):
    return api(node, 'listchannels')['channels']


def ready_channel(node, channel_id):
    return next((channel for channel in channels(node) if channel['channel_id'] == channel_id
                 and channel['ready'] and channel['is_usable'] and channel['status'] == 'Opened'), None)


def wallet_env(asset):
    env = os.environ.copy()
    env.update(RGB_NODE_URL='http://127.0.0.1:3101', RGB_NODE_TOKEN='', ALLOWED_ASSET_IDS=asset,
               AUTO_APPROVE_BELOW='1', MAX_SINGLE_PAYMENT='100', MAX_DAILY_SPEND='500',
               MAX_CARRIER_MSAT='3000000', WALLET_STATE_PATH=str(STATE / 'wallet-state.json'))
    return env


def write_wallet_env(asset):
    import shlex
    env = wallet_env(asset)
    keys = ('RGB_NODE_URL', 'RGB_NODE_TOKEN', 'ALLOWED_ASSET_IDS', 'AUTO_APPROVE_BELOW',
            'MAX_SINGLE_PAYMENT', 'MAX_DAILY_SPEND', 'MAX_CARRIER_MSAT', 'WALLET_STATE_PATH')
    (STATE / 'wallet.env').write_text(''.join(f'export {key}={shlex.quote(env[key])}\n' for key in keys))


def bootstrap():
    for node in PORTS:
        unlock(node)
    synced()
    if (STATE / 'bootstrap.json').exists():
        data = load('bootstrap.json')
        for node in PORTS:
            if api(node, 'nodeinfo')['pubkey'] != data['pubkeys'][node]:
                raise Failure('Node identity changed since bootstrap; reset this development environment')
        api('alice', 'connectpeer', {'peer_pubkey_and_addr': data['pubkeys']['bob'] + '@bob:9735'})
        for node in PORTS:
            wait_for(f'{node} existing channel', lambda node=node: ready_channel(node, data['channel_id']))
        write_wallet_env(data['asset_id'])
        say('Reusing issued demo asset and usable channel.')
        return data
    # No automatic retries of state-changing setup requests. Persist checkpoints.
    data = load('setup-progress.json') if (STATE / 'setup-progress.json').exists() else {}
    for node in PORTS:
        balance = api(node, 'btcbalance', {'skip_sync': False})
        if sum(part['future'] for part in balance.values()) == 0:
            if data.get(node + '_funding_requested'):
                raise Failure(f'{node} previous funding is uncertain; inspect the miner wallet before resetting')
            data[node + '_funding_requested'] = True
            save('setup-progress.json', data)
            address = api(node, 'address', {})['address']
            btc('sendtoaddress', address, '1')
            say(f'Funded {node} with 1 regtest BTC')
    mine(6)
    synced()
    for node in PORTS:
        try:
            api(node, 'createutxos', {'up_to': True, 'num': 10, 'size': 32500, 'fee_rate': 2, 'skip_sync': False})
        except ApiError as error:
            if error.name != 'AllocationsAlreadyAvailable':
                raise
    mine(6)
    synced()
    if 'asset_id' not in data:
        assets = api('alice', 'listassets', {'filter_asset_schemas': ['Nia']})['nia'] or []
        matches = [asset for asset in assets if asset['ticker'] == 'R402USD' and asset['name'] == 'RGB402 Demo Dollar' and asset['precision'] == 0]
        if len(matches) > 1:
            raise Failure('Multiple demo assets found; reset rather than guessing')
        asset = matches[0] if matches else api('alice', 'issueassetnia', {
            'amounts': [1000], 'ticker': 'R402USD', 'name': 'RGB402 Demo Dollar', 'precision': 0})['asset']
        data['asset_id'] = asset['asset_id']
        save('setup-progress.json', data)
    data['pubkeys'] = {node: api(node, 'nodeinfo')['pubkey'] for node in PORTS}
    api('alice', 'connectpeer', {'peer_pubkey_and_addr': data['pubkeys']['bob'] + '@bob:9735'})
    def candidate():
        found = [channel for channel in channels('alice') if channel['asset_id'] == data['asset_id']
                 and channel['peer_pubkey'] == data['pubkeys']['bob']]
        if len(found) > 1:
            raise Failure('Multiple matching channels found; refusing to guess')
        return found[0] if found else None
    if not candidate():
        if data.get('channel_open_requested'):
            raise Failure('Previous channel open is uncertain; inspect logs and reset explicitly')
        data['channel_open_requested'] = True
        save('setup-progress.json', data)
        api('alice', 'openchannel', {'peer_pubkey_and_opt_addr': data['pubkeys']['bob'] + '@bob:9735',
                                    'capacity_sat': 100000, 'push_msat': 10000000,
                                    'asset_id': data['asset_id'], 'asset_amount': 600,
                                    'push_asset_amount': 100, 'public': True, 'with_anchors': True})
    def funded():
        channel = candidate()
        if not channel or not channel['funding_txid']:
            return None
        if channel['ready']:
            return channel
        txout = btc('gettxout', channel['funding_txid'], 0)
        return channel if txout else None
    channel = wait_for('RGB channel funding transaction', funded)
    if not channel['ready']:
        mine(6)
        synced()
    data['channel_id'] = channel['channel_id']
    for node in PORTS:
        wait_for(f'{node} usable RGB channel', lambda node=node: ready_channel(node, data['channel_id']))
    save('bootstrap.json', data)
    write_wallet_env(data['asset_id'])
    say(f'Demo asset {data["asset_id"]}; channel {data["channel_id"]} is usable on both sides.')
    return data


def snapshots(asset):
    return {node: api(node, 'assetbalance', {'asset_id': asset}) for node in PORTS}


def wallet(command, value, asset):
    result = subprocess.run([str(ROOT / 'target/debug/wallet'), command, value],
                            cwd=ROOT, env=wallet_env(asset), text=True, capture_output=True)
    with (LOGS / 'wallet-audit.log').open('a') as stream:
        stream.write(result.stderr)
    if result.returncode:
        raise Failure(f'wallet {command} failed; see {LOGS / "wallet-audit.log"}')
    return json.loads(result.stdout)


def verify():
    attempt = load('attempt.json')
    request = attempt['request']
    def settled():
        status = wallet('status', request['payment_hash'], request['asset_id'])
        return status if status in ('Settled', 'Failed') else None
    status = wait_for('Wallet payment settlement (do not resubmit on timeout)', settled, 60)
    if status == 'Failed':
        raise Failure('Payment failed; reservation retained. Do not retry it.')
    def balances_changed():
        after = snapshots(request['asset_id'])
        before = attempt['before']
        amount = request['amount']
        return after if (before['alice']['offchain_outbound'] - after['alice']['offchain_outbound'] == amount
                         and after['bob']['offchain_outbound'] - before['bob']['offchain_outbound'] == amount) else None
    after = wait_for('Exact Alice/Bob RGB balance changes', balances_changed, 30)
    result = load('payment-result.json') if (STATE / 'payment-result.json').exists() else {}
    evidence = {'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'node_commit': PIN, 'rust_lightning_commit': LDK_PIN, 'asset_id': request['asset_id'],
                'ticker': 'R402USD', 'precision': 0, 'channel_id': attempt['channel_id'],
                'amount': request['amount'], 'carrier_msat': request['carrier_msat'],
                'payment_id': result.get('payment_id'), 'payment_hash': request['payment_hash'],
                'status': 'Settled', 'before': attempt['before'], 'after': after}
    save('acceptance.json', evidence)
    say('\nRGB Lightning payment settled.\n' + json.dumps(evidence, indent=2))


def status():
    say(compose('ps'))
    say(f'Bitcoin height: {btc("getblockcount")}; Electrs height: {index_height()}')
    for node in PORTS:
        try:
            info = api(node, 'nodeinfo', timeout=5)
            say(f'{node}: unlocked, pubkey={info["pubkey"]}, usable channels={info["num_usable_channels"]}')
        except ApiError as error:
            say(str(error))


def collect_logs():
    if not (STATE / 'compose.yaml').exists():
        return
    for service in ('bitcoind', 'electrs', 'proxy', 'alice', 'bob'):
        try:
            (LOGS / f'{service}.log').write_text(compose('logs', '--no-color', service) + '\n')
        except Failure:
            pass


def stop():
    # SIGTERM is handled gracefully by upstream; no volume deletion here.
    compose('stop', 'alice', 'bob')
    compose('stop', 'electrs', 'proxy', 'bitcoind')
    collect_logs()
    say('Regtest services stopped; all development state preserved.')


def reset(yes):
    if not yes:
        raise Failure(f'Reset deletes only {STATE}. Re-run reset.sh --yes to confirm. Upstream checkout is preserved.')
    say(f'Deleting only development regtest state: {STATE}')
    docker_ready()
    if (STATE / 'compose.yaml').exists():
        compose('down')
    data = STATE / 'data'
    if data.exists():
        # Fixed mount and fixed children only; handles files from upstream UID 1000.
        run(['docker', 'run', '--rm', '--user', '0:0', '--entrypoint', '/bin/rm',
             '--mount', f'type=bind,src={data},dst=/state', DEBIAN,
             '-rf', '--', '/state/alice', '/state/bob', '/state/core', '/state/index'])
    if STATE.exists():
        shutil.rmtree(STATE)
    say('Only this project\'s local regtest state was removed.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['start', 'stop', 'reset', 'status', 'bootstrap', 'demo', 'verify'])
    parser.add_argument('--yes', action='store_true', help='confirm destructive development reset only')
    args = parser.parse_args()
    safe_paths()
    import fcntl
    (ROOT / '.var').mkdir(exist_ok=True)
    operation_lock = (ROOT / '.var/regtest-operation.lock').open('a')
    try:
        fcntl.flock(operation_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        raise Failure('Another regtest command is running; wait for it to finish') from error
    if args.command == 'reset':
        reset(args.yes)
        return
    if args.yes:
        raise Failure('--yes is only valid for reset; payment approval must be interactive')
    LOGS.mkdir(parents=True, exist_ok=True)
    try:
        if args.command == 'start': start()
        elif args.command == 'stop': stop()
        elif args.command == 'status': status()
        elif args.command == 'bootstrap':
            start()
            bootstrap()
        elif args.command == 'verify': verify()
        elif args.command == 'demo': demo()
    finally:
        collect_logs()


# demo() is defined below alongside its interactive wallet invocation.

def payment_result(output):
    decoder = json.JSONDecoder()
    for position, character in enumerate(output):
        if character != '{':
            continue
        try:
            value, _ = decoder.raw_decode(output[position:])
        except ValueError:
            continue
        if isinstance(value, dict) and {'payment_id', 'payment_hash', 'status'} <= value.keys():
            return value
    return None


def demo():
    say('[1/5] Starting pinned infrastructure and nodes')
    start()
    say('[2/5] Bootstrapping funded wallets and usable RGB channel')
    setup = bootstrap()
    run(['cargo', 'build', '--locked', '-p', 'buyer-agent', '--bin', 'wallet'], log='wallet-build.log')
    if (STATE / 'attempt.json').exists():
        say('An acceptance attempt already exists; verifying it without another submission.')
        verify()
        return
    asset = setup['asset_id']
    say('[3/5] Creating Bob invoice; decoding and preparing through our wallet')
    before = snapshots(asset)
    invoice = api('bob', 'lninvoice', {'amt_msat': 3000000, 'expiry_sec': 3600,
                                     'asset_id': asset, 'asset_amount': 5,
                                     'description': 'RGB402 Demo Dollar acceptance payment'})['invoice']
    save('invoice.json', {'invoice': invoice})
    request = wallet('decode', invoice, asset)
    plan = wallet('prepare', invoice, asset)
    save('wallet-decode.json', request)
    save('wallet-prepare.json', plan)
    if request['asset_id'] != asset or request['amount'] != 5 or plan['policy']['decision'] != 'require_approval':
        raise Failure('Unexpected invoice or policy decision; no payment submitted')
    save('attempt.json', {'request': request, 'before': before, 'channel_id': setup['channel_id']})
    say('[4/5] Paying through WalletService — review the exact plan and answer y/N')
    with (LOGS / 'wallet-audit.log').open('a') as audit:
        process = subprocess.Popen([str(ROOT / 'target/debug/wallet'), 'pay', invoice],
                                   cwd=ROOT, env=wallet_env(asset), text=True,
                                   stdin=None, stdout=subprocess.PIPE, stderr=audit)
        output = []
        while True:
            character = process.stdout.read(1)
            if not character:
                break
            output.append(character)
            sys.stdout.write(character)
            sys.stdout.flush()
        returncode = process.wait()
    output = ''.join(output)
    (STATE / 'payment-output.txt').write_text(output)
    result = payment_result(output)
    if returncode:
        raise Failure('Payment command failed or is uncertain; do not resubmit. Run verify.sh and inspect wallet-audit.log.')
    if result is None:
        if 'Payment cancelled.' in output:
            (STATE / 'attempt.json').unlink()
            say('Cancelled; no payment submitted.')
            return
        raise Failure('Missing payment result; preserve state and inspect logs before doing anything else')
    save('payment-result.json', result)
    say('[5/5] Verifying settled status and exact RGB balance changes')
    verify()


if __name__ == '__main__':
    try:
        main()
    except (Failure, OSError, ValueError, KeyError) as error:
        print(f'ERROR: {error}', file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print('Interrupted. State preserved; query status before retrying any payment.', file=sys.stderr)
        sys.exit(130)
