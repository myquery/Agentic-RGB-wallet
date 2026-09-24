// Learn more https://docs.expo.dev/guides/customizing-metro
const { getDefaultConfig } = require('expo/metro-config')
const path = require('path')
const fs = require('fs')

const config = getDefaultConfig(__dirname)

// The worklet bundle lives under `.wdk-bundle/wdk-worklet.bundle.js`.
// Metro's default `blockList` ignores dotfile folders — explicitly allow
// watching it so `import bundle from './.wdk-bundle/wdk-worklet.bundle.js'`
// resolves. The file itself is a normal `.js` that does
// `module.exports = "<~6 MB string>"`, which Metro parses fine.
config.watchFolders = [
  ...(config.watchFolders ?? []),
  path.resolve(__dirname, '.wdk-bundle'),
]

// Metro 0.83 has a resolver bug where a package's `main` field that
// already includes the `.js` extension (e.g. `"main": "index.js"`)
// triggers an extension-appending probe (`index.js.android.js`, …)
// instead of resolving the literal path. Bare-runtime packages pulled
// in by the HRPC stack hit this — fall back to the literal `main` file
// when Metro's default resolver throws PackageResolutionError.
config.resolver = {
  ...(config.resolver ?? {}),
  unstable_enablePackageExports: true,
  resolveRequest: (context, moduleName, platform) => {
    try {
      // `context.resolveRequest` is Metro's default resolver here (Metro
      // swaps it in so user hooks can delegate to it without recursing).
      return context.resolveRequest(context, moduleName, platform)
    } catch (err) {
      const msg = err && err.message
      if (typeof msg === 'string' && msg.includes('`main` module field')) {
        const m = msg.match(/`main` module field that could not be resolved \(`([^`]+)`/)
        const mainFile = m && m[1]
        if (mainFile && fs.existsSync(mainFile)) {
          return { type: 'sourceFile', filePath: mainFile }
        }
      }
      throw err
    }
  }
}

module.exports = config
