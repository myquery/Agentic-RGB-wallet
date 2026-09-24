# Android WDK capsule compatibility spike

This isolated Expo/React Native application tests whether the pinned WDK/UTEXO RGB Lightning stack can host one wallet directly on Android. It does not import Luma production code or use Alice, Bob, or Carol state.

## Selected stack

The dependency tree is frozen in `package-lock.json`. The key pins are WDK `1.0.0-beta.14`, WDK React Native Core `1.0.0-beta.21`, WDK RGB Lightning `0.1.0-beta.15`, RGB Lightning Bare `0.1.0-beta.15`, Bare Kit `0.15.5`, Expo `54.0.33`, React Native `0.81.5`, and Node.js 22.

The UTEXO beta.15 Android x64 artifact is 158,991,120 bytes and has SHA-256 `2e8c740bf4402da30739d97ab30bd8cded737284ee60b5fbde7e8503757cb200`. The official release says it embeds `rgb-lightning-node v0.10.0-beta.3`.

## Reproduce the tested build

```bash
nvm use 22
npm ci --ignore-scripts --legacy-peer-deps
./scripts/fetch-android-x64-native.sh
npm run bundle:wdk
npx expo prebuild --platform android --no-install
cd android
./gradlew assembleRelease
```

The bundle is deliberately limited to `android-x64`, matching the tested API 36 emulator. A physical ARM device requires the matching pinned beta.15 artifact and a separate validation run.

## Current result

The release APK builds and launches, but WDK cannot start its Bare worklet. Bare aborts with:

```text
ADDON_NOT_FOUND: Cannot find addon '.' imported from
file:///wdk-worklet.bundle/node_modules/bare-channel/binding.js
```

The APK contains `lib/x86_64/libbare-channel.5.3.0.so`, and the generated bundle resolves the module to `linked:libbare-channel.5.3.0.so`. Constraining the bundle to `android-x64` does not change the failure. Bare Kit `0.14.5`, the minimum accepted by WDK React Native Core beta.21, also fails during addon resolution (`bare-type`).

Because the failure occurs before wallet creation, later BTC, RGB, channel, payment, persistence, and VSS tests are blocked. See `docs/architecture/android-wdk-compatibility-spike.md` for the full assessment.
