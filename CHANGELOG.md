# Changelog

## 1.5.1 - 2025-12-04

### <!-- 02 -->🐛 Bug fixes

- Ignore filters in edit mode ([ec9cb40](https://github.com/desbma/rsop/commit/ec9cb40a2fbebe896585d1abcacbc0d53f6c1722) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Update lints ([89b3d88](https://github.com/desbma/rsop/commit/89b3d88fcb6e6855fc020eebeae1286823fce08f) by desbma)

______________________________________________________________________

## 1.5.0 - 2025-09-16

### <!-- 01 -->💡 Features

- Try all handler candidates by extension before trying by MIME ([7a627cc](https://github.com/desbma/rsop/commit/7a627cccdc95b1d89c2b62952c561725c2c15d18) by desbma)
- Update advanced config ([701d548](https://github.com/desbma/rsop/commit/701d548471e8b963c461f1d3c293ff370281b566) by desbma)
- ci: Add cargo audit ([7020481](https://github.com/desbma/rsop/commit/702048184f14336735c5d48e2b511fb0bc63491b) by desbma)
- Support %t and %T substitutions ([ae9957d](https://github.com/desbma/rsop/commit/ae9957d853a3fad68df0b7b60e80a1111ae668ce) by desbma)

### <!-- 02 -->🐛 Bug fixes

- build: Remove unmaintained prettier pre-commit hook ([8c4df5c](https://github.com/desbma/rsop/commit/8c4df5c398a7640d02382dce5df30ebe178ef32a) by desbma)

### <!-- 04 -->📗 Documentation

- README: Add yazi setup instructions ([36f38a8](https://github.com/desbma/rsop/commit/36f38a8cc6035992f38dfa02f3da36b75a66deac) by desbma)
- README: Update yazi setup instructions ([910879d](https://github.com/desbma/rsop/commit/910879d6628fdaa01305393465440720404fc5c9) by desbma)

### <!-- 06 -->🚜 Refactor

- Replace lazy_static by LazyLock ([db77aca](https://github.com/desbma/rsop/commit/db77acac28928ff7317e36b68ddc441f16d6e30e) by desbma)
- Lazy anyhow context ([9d236a1](https://github.com/desbma/rsop/commit/9d236a10c9b34d41916beba0269098fb02e6f95e) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Update release script ([bf507d8](https://github.com/desbma/rsop/commit/bf507d805f04c4c34e9207c00769cfb41ec5d603) by desbma)
- Update dependencies ([f3942a3](https://github.com/desbma/rsop/commit/f3942a3e99165fbebeb5b77464ae60a689150df6) by desbma)
- Update pre-commit hooks ([db71130](https://github.com/desbma/rsop/commit/db71130584479a0ce877941f6309b69eb59e5f7e) by desbma)
- Enable more lints ([16c4977](https://github.com/desbma/rsop/commit/16c497799c9708208d5172e649a6619c50b7e21b) by desbma)
- Update dependencies ([81664b4](https://github.com/desbma/rsop/commit/81664b414ed4d4ac8ade3bae786fc52e47bc3432) by desbma)
- Update lints ([900915a](https://github.com/desbma/rsop/commit/900915a6545f4156495695879aaea71adea08162) by desbma)
- Update changelog template ([632b07d](https://github.com/desbma/rsop/commit/632b07d2bc0cc66aa92a1127d395270d9b5e8002) by desbma)

______________________________________________________________________

## 1.4.2 - 2024-06-07

### <!-- 02 -->🐛 Bug fixes

- Replace buggy/abandoned term size crate ([79ab0a5](https://github.com/desbma/rsop/commit/79ab0a54b21d3c662f7c63bc937b52df34c346b5) by desbma)
- Shlex deprecation warning ([f01776e](https://github.com/desbma/rsop/commit/f01776e741d97c9a7f7f440d4432e89334a0d62a) by desbma)

### <!-- 04 -->📗 Documentation

- README: Rename AUR package ([f6bebe8](https://github.com/desbma/rsop/commit/f6bebe8fb0c57ea22c2230890a4df51a783c89aa) by desbma)

### <!-- 06 -->🚜 Refactor

- Remove dedicated splice code path ([5469661](https://github.com/desbma/rsop/commit/5469661d44089d1bc81abc00b066be4bac2adc96) by desbma)

______________________________________________________________________

## 1.4.1 - 2024-01-19

### <!-- 01 -->💡 Features

- Update advanced config ([3a8e32c](https://github.com/desbma/rsop/commit/3a8e32c67dd3722a11e01748b6bf2ff76e373d39) by desbma)

### <!-- 02 -->🐛 Bug fixes

- Mime iteration for piped data ([f69abc2](https://github.com/desbma/rsop/commit/f69abc24cab36a448c7c84e02f9735d4044d4832) by desbma)
- Try alternate mode handlers when piped too ([b6d7b21](https://github.com/desbma/rsop/commit/b6d7b214afef5aaf4e897be77133f89691219064) by desbma)

______________________________________________________________________

## 1.4.0 - 2023-11-20

### <!-- 01 -->💡 Features

- More general support for MIME prefix match ([cb724f2](https://github.com/desbma/rsop/commit/cb724f2d62ff664d22cdaad6fbfcbb4f5fba0019) by desbma)
- Update/improve advanced config ([0e7fc10](https://github.com/desbma/rsop/commit/0e7fc10b8778e8c2ce7c0e0c3dd3409fe6161e9c) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Move from structopt to clap ([e1a54ef](https://github.com/desbma/rsop/commit/e1a54ef43e81a195c58df28033505d49d78d541f) by desbma)

______________________________________________________________________

## 1.3.1 - 2023-10-10

### <!-- 02 -->🐛 Bug fixes

- %m substitution sometimes skipped ([77c6e27](https://github.com/desbma/rsop/commit/77c6e27fcd2fbb95186416528bbd5049bbdb0389) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Lint ([b1d628d](https://github.com/desbma/rsop/commit/b1d628d110f706415873341d82f26897493e07b9) by desbma)

______________________________________________________________________

## 1.3.0 - 2023-04-23

### <!-- 01 -->💡 Features

- Support edit action ([63106c2](https://github.com/desbma/rsop/commit/63106c2103551ea04b4c411067614217dcc1c0d7) by desbma)

### <!-- 02 -->🐛 Bug fixes

- Build on MacOS ([4113b8d](https://github.com/desbma/rsop/commit/4113b8d2a7bd1f7c1ec990f073c7485e511f3375) by desbma)

### <!-- 04 -->📗 Documentation

- README: Fix ranger scope.sh instructions ([11e87e3](https://github.com/desbma/rsop/commit/11e87e3fafa355c91b229b59229e8d7bf42ab683) by desbma)

### <!-- 05 -->🧪 Testing

- Add macos-latest machine for ci test (#4) ([280cff6](https://github.com/desbma/rsop/commit/280cff6925c49414cb05b5ecc8f8a717f555f350) by Heechul Ryu)

### <!-- 10 -->🧰 Miscellaneous tasks

- Lint ([e6283f2](https://github.com/desbma/rsop/commit/e6283f2f259cef7a6515d303b4920b5a99ed00fc) by desbma)
- Lint ([e99819d](https://github.com/desbma/rsop/commit/e99819d48bb1f2fdeebc3dfe3c2fbc49465dbf32) by desbma)
- Lint ([7ffa5a9](https://github.com/desbma/rsop/commit/7ffa5a911b85121ed72b2d10bb7d065b0baa2212) by desbma)

______________________________________________________________________

## 1.2.2 - 2022-10-31

### <!-- 01 -->💡 Features

- Update advanced config ([b29e7ca](https://github.com/desbma/rsop/commit/b29e7caa98193082a9a7a2e7755d340d279fc735) by desbma)
- Improve error handling in worker threads ([717bc0f](https://github.com/desbma/rsop/commit/717bc0f5a94ce4764f1a318b7673dfcad8a4e848) by desbma)
- Update advanced config ([22aa982](https://github.com/desbma/rsop/commit/22aa982477b9e84d06cab3f978461be6e47a11cb) by desbma)
- Build with full LTO + strip ([ef508b2](https://github.com/desbma/rsop/commit/ef508b2c59af741e639b115493eb9d94479c2e44) by desbma)
- Add RSOP_INPUT_IS_STDIN_COPY·env·var. ([edc05d2](https://github.com/desbma/rsop/commit/edc05d2af98b6771e124a31b74b5de4f2e383caa) by desbma)

### <!-- 02 -->🐛 Bug fixes

- Archive open handler in advanced config example ([3cfc065](https://github.com/desbma/rsop/commit/3cfc065de61006d5be86b41046e8440326653dc4) by desbma)
- Disable default nix features ([fa951ac](https://github.com/desbma/rsop/commit/fa951acaca91b2e6bdda751df3ea1ff0ce4f122f) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Lint ([3763f39](https://github.com/desbma/rsop/commit/3763f39d6f432c4299235a9ffb8888386aa86f49) by desbma)
- Update dependencies ([4a08932](https://github.com/desbma/rsop/commit/4a0893275497a0c41f5701f3649680147d2aaa6c) by desbma)
- Rename release script ([15b7c07](https://github.com/desbma/rsop/commit/15b7c07415d75cc2aafdcb9d44a9b1a642e5dd49) by desbma)

______________________________________________________________________

## 1.2.1 - 2022-03-30

### <!-- 01 -->💡 Features

- Improve reporting of rsi errors ([ddf2be1](https://github.com/desbma/rsop/commit/ddf2be137132c1e7c0801c1e302804154fa9fc64) by desbma)
- Update advanced config ([85d9b39](https://github.com/desbma/rsop/commit/85d9b39d6424dec6a39606e3016cf2dc38d4677e) by desbma)
- Update advanced config ([5b43516](https://github.com/desbma/rsop/commit/5b43516226bfc7f2b8bba7693e2b0eaaf20f80a0) by desbma)
- Add check for invalid config with no_pipe=false and multiple input patterns ([3a0d594](https://github.com/desbma/rsop/commit/3a0d59476f48e56e47f7ca7d0a21d01294544485) by desbma)

### <!-- 02 -->🐛 Bug fixes

- Run check/tests in release script ([90d3c0b](https://github.com/desbma/rsop/commit/90d3c0ba381135c0384fcae79b0a5a0eb208a47b) by desbma)

### <!-- 10 -->🧰 Miscellaneous tasks

- Lint ([1b40e2f](https://github.com/desbma/rsop/commit/1b40e2fefe35d5002b6083cbfd0ab5eaeecbd28e) by desbma)

______________________________________________________________________

## 1.2.0 - 2022-01-08

### <!-- 01 -->💡 Features

- Support matching by double extensions ([7be9be0](https://github.com/desbma/rsop/commit/7be9be0fafdf405479f425d04e0c70b36d4769fd) by desbma)
- Ensure extension matching is case insensitive ([42cd35d](https://github.com/desbma/rsop/commit/42cd35d704807d5b111e66a4631734117ec16763) by desbma)
- Update advanced config ([39ca703](https://github.com/desbma/rsop/commit/39ca703bffc00abbea4a704321afefc73da10df8) by desbma)

______________________________________________________________________

## 1.1.2 - 2021-12-29

### <!-- 01 -->💡 Features

- Improve error display for common errors ([9a07d59](https://github.com/desbma/rsop/commit/9a07d59ee21f2ba1a01e380df9703e3df91f0eba) by desbma)
- Improve error display for common errors, take 2 ([f1f558c](https://github.com/desbma/rsop/commit/f1f558c45994d4fc48b4c05d7e40cc2c3b59020d) by desbma)

### <!-- 04 -->📗 Documentation

- Fix README typo ([511d1b4](https://github.com/desbma/rsop/commit/511d1b4bcef752ae99931f3b1c0f3bb74be04b22) by desbma)
- Add AUR package link in README ([cd0e67f](https://github.com/desbma/rsop/commit/cd0e67ff4469f653cf720040eecb266c6d1398c0) by desbma)

### <!-- 06 -->🚜 Refactor

- Remove better-panic ([901c76f](https://github.com/desbma/rsop/commit/901c76fd0072f44fbe562c11ff3409dedf189621) by desbma)

______________________________________________________________________

## 1.1.1 - 2021-12-05

### <!-- 02 -->🐛 Bug fixes

- Mode detection with absolute path ([16e7e5e](https://github.com/desbma/rsop/commit/16e7e5ece76e06a07ba6e0c50b801b9fc2f82daf) by desbma)

______________________________________________________________________

## 1.1.0 - 2021-09-27

### <!-- 01 -->💡 Features

- Support file:// url prefix ([b0175ac](https://github.com/desbma/rsop/commit/b0175ac286e4c1abf0cfb395d6492acb543a0eef) by desbma)
- Dynamically compute pipe peek size from system page size ([abad351](https://github.com/desbma/rsop/commit/abad351c7f3bc300e889b06dcd546aeb713235bf) by desbma)
- Support %m substitution in command for MIME type ([17462dc](https://github.com/desbma/rsop/commit/17462dc1e020862dc76fd6db26169fa8baff7218) by desbma)
- Add no_pipe option to use temp file if handler does not support reading from stdin ([c6697ba](https://github.com/desbma/rsop/commit/c6697ba5f55d21e99619958f29fc7b2cfd7c9f68) by desbma)
- URL handlers for xdg-open compatibility ([809e2e0](https://github.com/desbma/rsop/commit/809e2e08b9a9313a9128383716aedee70bfca3c6) by desbma)

### <!-- 02 -->🐛 Bug fixes

- Add config check for handlers with both 'no_pipe = true' and 'wait = false' ([c75f23f](https://github.com/desbma/rsop/commit/c75f23fa3810d2999d96f9544ce3acb52ae6a313) by desbma)
- Incompatible flags in advanced config ([2ac6ac1](https://github.com/desbma/rsop/commit/2ac6ac1308e79dbdb325e806cdb711c6f76ce508) by desbma)

### <!-- 04 -->📗 Documentation

- Use git-cliff to generate changelog ([62ffa0a](https://github.com/desbma/rsop/commit/62ffa0a4e8ad2aef8adf83c6430f2b64329836a0) by desbma)

### <!-- 05 -->🧪 Testing

- Add tests for default and advanced config ([3084fb6](https://github.com/desbma/rsop/commit/3084fb6fb4969a6aca0cb2c1b1ca04096c25ee15) by desbma)
- Test for smallest possible config ([23f9232](https://github.com/desbma/rsop/commit/23f92321614413e897774cc7c731ef4023e992df) by desbma)

### <!-- 06 -->🚜 Refactor

- Remove duplicate/hardcoded strings in mode handling ([174a5c6](https://github.com/desbma/rsop/commit/174a5c6edeee56cd624509ee4059fb33b33da9da) by desbma)
- Factorize pattern substitution code ([a821d16](https://github.com/desbma/rsop/commit/a821d160a25193f575d8cc58f6b2b70b8aa91c85) by desbma)

### Config

- Add application/x-cpio MIME in advanced config + reformat long lists ([4ee4a3e](https://github.com/desbma/rsop/commit/4ee4a3e7a6ae5baa227cbc6a428abcfa684cb448) by desbma)
- Fix some handlers in advanced config when piped ([c032fbc](https://github.com/desbma/rsop/commit/c032fbc08544c65b5111584052dc14739733cb1c) by desbma)
- Add application/x-archive MIME in advanced config ([c16c671](https://github.com/desbma/rsop/commit/c16c6713143447f642e3d5d3f02b99ca4c258328) by desbma)
- Fix some more handlers in advanced config when piped ([be52ac0](https://github.com/desbma/rsop/commit/be52ac015f0ecd3d48fc602e76a317ac797b15d2) by desbma)
- Add openscad preview in advanced config ([f27f0f7](https://github.com/desbma/rsop/commit/f27f0f740a2f1a03786fb26d67d5816f267babd6) by desbma)
- Fix one more handler in advanced config when piped ([12b369d](https://github.com/desbma/rsop/commit/12b369d7567c80caaeeb0fec03e5f7baff24e9d6) by desbma)
- Fix remaining handlers in advanced config when piped + remove redundant flags ([87b292e](https://github.com/desbma/rsop/commit/87b292e7f1cc868055537dc8877ad151521245d0) by desbma)
