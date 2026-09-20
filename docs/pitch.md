Luma — Your wallet. Your words. Your approval.
“Buy me a coffee from Carol.”
That sounds simple. But an AI agent needs to find the right merchant, identify the product, verify its price, prepare a payment, and know whether it actually settled.
And throughout that process, you should stay in control of your money.
We built Luma, an agentic Bitcoin and RGB wallet that turns a natural-language request into a verified payment.
Here’s how it works.
Carol enables merchant mode in her wallet and publishes Coffee for five R402USD, our demo RGB asset. Alice asks her wallet agent to buy it.
The agent discovers Carol’s catalog and requests an order. Carol’s service sets the price and generates an invoice. Luma’s deterministic wallet code checks the asset, amount, expiry, balance, and spending policy.
Then the application presents the exact payment for Alice to approve.
The AI can propose a payment. It cannot manufacture approval.
After Alice approves, Luma sends the RGB payment over Lightning. The node confirms settlement, Carol’s order becomes paid, and Alice gets a receipt.
We demonstrated this on a real three-node regtest setup: Alice bought Coffee for five R402USD, and Bob bought a Sandwich for eight. Both payments settled, and Carol received thirteen units.
Luma also supports human BTC payments and an L402 workflow for purchasing protected digital resources. These use separate policies through the existing wallet infrastructure.
Our core contribution is the boundary between AI convenience and payment authority: bound approvals, durable reservations, duplicate protection, and node-verified completion.
Today, Luma is a working regtest prototype—not a production wallet. Our next step is making merchant onboarding and payment recovery ready for broader testing.
Luma makes payments conversational, while keeping authorization in your hands.
For the live demo, use this sequence:
1. Show Alice’s balance and Carol’s published Coffee.
2. Ask Alice’s agent: “What does Carol sell?”
3. Ask: “Buy one Coffee from Carol.”
4. Highlight the application-controlled approval.
5. Approve once, then show authoritative settlement.
6. Show Carol’s paid order and updated balance.
7. Ask Alice: “Show my Coffee receipt.”
If settlement is pending, show that honestly and query the existing payment—don’t create another purchase.