module.exports = {
  networks: {
    'rgb-lightning': { package: '@utexo/wdk-rgb-lightning' }
  },
  options: {
    targets: ['android-x64'],
    platforms: ['android'],
    linkAddons: true
  }
}
