# Demo liquidity requirements

Amounts below refer to Alice paying Bob. They are separate from policy budgets.
Use `scripts/demo/check.py --snapshot <new-file>` to record current values.

| Resource | Requirement and authoritative field |
| --- | --- |
| Native BTC outbound | A usable direct channel must permit the exact amount between `next_outbound_htlc_minimum_msat` and `next_outbound_htlc_limit_msat`. Scenario B needs 10,000 msat; a new C needs 3,000 msat. The wallet's conservative capacity is the largest next-HTLC limit, not sum of on-chain BTC. |
| Native BTC inbound | Bob needs remote-side capacity (`inbound_balance_msat`) on the corresponding usable channel. On-chain funds do not immediately provide Lightning receive capacity. Direction and negotiated minimum matter. |
| RGB asset | Alice must hold the current R402USD asset with precision zero and at least 5 `offchain_outbound` units. UI `holdings[].outbound` maps this field. Bob must recognize the same asset ID. On-chain spendable RGB is reported separately and is not Lightning outbound. |
| RGB BTC carrier | RGB invoices use 3,000,000 msat (3,000 sats) of BTC carrier plus node-managed routing fees. The RGB channel must support this HTLC and Bob needs corresponding inbound capacity. RGB balance alone is insufficient. |

For a fresh A+B+C sequence, budget at least 3,013 sats of principal/channel
movement plus fee headroom, and 5 RGB units. Preflight checks exact single HTLC
limits and Bob's aggregate receive capacity, not a guarantee of the combined
route sequence. Recheck between scenarios; reserves, HTLC slots, fees and channel
state can change. D should consume no new payment capacity; rejecting E consumes
no payment principal. Failure/uncertainty reservations still count toward policy.

The Bob-funded BTC-only channel permits small Bob → Alice transfers (minimum
observed 1 msat). Its opposite direction may have zero outbound and a 3,000-sat
minimum. Alice's original RGB channel also carries native BTC toward Bob.
Do not assume a channel minimum is symmetric or that on-chain funding can be
spent directly over Lightning. See [small-payment channel history](acceptance/small-btc-channel.json).
No rebalancing, channel opening or payment is performed by the final helpers.
