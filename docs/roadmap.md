# Luma roadmap

Luma is a working regtest prototype for conversational, policy-controlled
payments. It already demonstrates RGB over Lightning payments, BTC Lightning
payments, merchant orders, L402 machine purchases, durable reservations,
application-owned approval, and node-verified settlement.

The roadmap extends that working boundary without giving a model authority to
spend money.

## 1. Mobile wallet foundation

Evaluate **Tether WDK** as the mobile-wallet foundation for Luma. The goal is
to reduce onboarding friction through a production-oriented wallet SDK, mobile
key and wallet lifecycle management, and a maintainable path for supported
payment integrations.

RGB Lightning node operation, settlement evidence, spending policy, and the
approval boundary remain Luma responsibilities. WDK adoption should be proven
through an adapter and mobile acceptance tests before it is presented as a
managed RGB Lightning-node solution.

## 2. A personal shopping and payment agent

Replace the cloud-model-only experience with an optional personal model that
can understand a user's preferred merchants, product language, budgets, and
approval habits. A small local or privacy-conscious model, including a
Muse Spark-style approach where suitable, could provide a more personal shopping
experience without sending every preference to a hosted provider.

Personalization may rank products, draft requests, and explain choices. It
cannot create approval, alter merchant prices, bypass spending limits, or submit
a payment outside the deterministic wallet flow.

## 3. Connect existing merchant catalogs

Add adapters for storefronts such as **Shopify** and **WooCommerce**. Luma will
use their catalog URLs and product metadata for discovery, then request a
merchant-controlled order from an L402-compatible commerce endpoint.

The storefront remains the source of truth for availability, product IDs, and
price. Before a payment is prepared, Luma must bind the exact product, quantity,
asset, amount, invoice expiry, and merchant domain into an immutable plan. A
catalog URL alone never authorizes a payment.

## 4. L402 for every merchant and service

Make L402 configuration available to standalone merchants as well as hosted
services. A merchant should be able to put physical goods, digital products,
compute, APIs, subscriptions, and machine-accessible services behind a payment
gate, publish a catalog, and issue an order or resource response after verified
settlement.

Luma will keep separate policies for RGB transfers, human BTC payments, and
automatic machine purchases. Merchants set offers and payment requests; users
set limits and retain final approval where policy requires it.

## Delivery sequence

1. Stabilize the current demo, onboarding, recovery, and merchant discovery.
2. Build catalog adapters and an L402 merchant configuration flow.
3. Add the WDK mobile-wallet integration behind tested wallet adapters.
4. Introduce optional personal-model personalization while preserving the same
   deterministic authorization and settlement controls.
