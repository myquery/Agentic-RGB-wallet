# Live recipient agent attempt — stopped before preparation

Production discovery-only readiness passed; both nodes were unlocked at chain
height 121 with usable channels. Fresh outbound balances were Alice 500 and Bob
100. Effective API AUTO_APPROVE_BELOW was 1, so amount >= 1 would require approval.

The exact request was: `Pay 5 R402USD to alice@3f0e-102-88-113-62.ngrok-free.app.`
It was submitted once through the existing PWA agent message endpoint.
The application returned: `Provide a supported asset ID and positive integer amount`.
The model then asked for clarification. No pending plan or approval prompt exists.

The error is the recipient contract-construction error. The running API binary
was built before the approved tilde correction; the newer standalone preflight
binary had passed the same asset. This indicates stale deployed application code.
No behavior was changed and no second model attempt was made. Rebuild/restart the
PWA API with the corrected existing source before a separately resumed attempt.
Do not substitute a direct invoice or treat model text as approval.

No acquisition, reservation, payment submission or status poll occurred in this
attempt. No serialized successful preparation observation exists to measure.
The session API supplies user-facing events, not raw model tool calls; the tool
name is inferred from its unique error site, not claimed as a captured wire trace.
See the JSON record for the sanitized transcript and fresh balance snapshots.

After-state queries confirmed Alice 500 and Bob 100 outbound RGB, unchanged.
Validation: 36 recipient regressions, final formatting/strict Clippy and frontend
production build pass. The temporary discovery-only runner was removed.
