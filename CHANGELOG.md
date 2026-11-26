# Changelog

## [0.4.0](https://github.com/marcosmoura/deck/compare/v0.3.0...v0.4.0) (2026-11-26)


### Features

* agent activity in the sidebar (DECK-50) ([3274f28](https://github.com/marcosmoura/deck/commit/3274f287cdffef4ebae4ba010af8444d45c37169))
* keep agent activity lines up to 400 characters ([5f3e5f1](https://github.com/marcosmoura/deck/commit/5f3e5f142e4b7862fb202c9667febb36ebb3588c))
* notification clicks focus the terminal and raise its window (DECK-52) ([a962983](https://github.com/marcosmoura/deck/commit/a962983e344775fec4f416735b35838bd1e376c6))
* notifications that open their panel (DECK-51) ([64a85c1](https://github.com/marcosmoura/deck/commit/64a85c13c7dace0d0488186bee251180db277414))
* security, privacy and performance reviews (DECK-26) ([9aedd08](https://github.com/marcosmoura/deck/commit/9aedd0866431abf6f738911b6d11bab146e30ecd))
* show a loader in the sidebar while a command runs ([904a92d](https://github.com/marcosmoura/deck/commit/904a92d01a96a14f4de94ae78ba5d4d966f43c9a))
* **tui:** draw the loader as eight fading spokes at 8 frames a second ([ceff9b2](https://github.com/marcosmoura/deck/commit/ceff9b2867cfcaf0d3145e6997dcc242ac099b13))
* **tui:** show cut sidebar text in full on hover ([8377d77](https://github.com/marcosmoura/deck/commit/8377d772f769840b1cf1dbc4576b7132fc9aa670))
* **tui:** wrap the sidebar tooltip to five lines ([48225c5](https://github.com/marcosmoura/deck/commit/48225c50809d84210cc832037795fafdfec7d926))


### Bug Fixes

* **cli:** send codex events to the codex terminal ([7fc63ad](https://github.com/marcosmoura/deck/commit/7fc63ad99ff075b34183488436fb2835546ef7a6))
* **cli:** show the daemon's refusal when it hangs up before the request ([227a1b0](https://github.com/marcosmoura/deck/commit/227a1b0b94a6c028e3b131607469a3cb5e6ba370))
* **core:** drop Codex's folder from panel names ([0f0f070](https://github.com/marcosmoura/deck/commit/0f0f070028f24426b72f79f39f28ddf3b6f8570a))
* **core:** drop OpenCode's OC prefix from panel names ([995bc41](https://github.com/marcosmoura/deck/commit/995bc4112261a01cadd20e4dd041a913a5c23b88))
* **daemon:** reap the opencode listings a timeout or a closed connection stops ([56892d3](https://github.com/marcosmoura/deck/commit/56892d318e15354802229e8c08dbe50741795e6e))
* opencode status from v2.4 turn events ([a387276](https://github.com/marcosmoura/deck/commit/a387276840b9fd7a8bda6c13e2bed1bbf451fdf6))
* **tui:** draw each frame inside synchronized output ([16f2328](https://github.com/marcosmoura/deck/commit/16f23284765e32e11245e250107dc6a290b50dcf))

## [0.3.0](https://github.com/marcosmoura/deck/compare/v0.2.0...v0.3.0) (2026-11-25)

### Features

- **cli:** add shell completion ([d002f75](https://github.com/marcosmoura/deck/commit/d002f75a01bb1c31ed37ab65f775f9389e49f959))
- client exits when its terminal closes (DECK-39) ([802ada4](https://github.com/marcosmoura/deck/commit/802ada424869a72d1e9885a8d7d6c50cb2b065c1))
- **cli:** install the agent skill with deck skills (DECK-31) ([02377ec](https://github.com/marcosmoura/deck/commit/02377ec49487c0cc255b3d537762b32958db93fb))
- collapsed sidebar (DECK-48) ([b746233](https://github.com/marcosmoura/deck/commit/b746233f360347cdc96e8d9785864a0bf0a5214f))
- live upgrade (DECK-21) ([e46b102](https://github.com/marcosmoura/deck/commit/e46b102cc0e311663fd0c17f1310eeaa8341cb2d))

## [0.2.0](https://github.com/marcosmoura/deck/compare/v0.1.0...v0.2.0) (2026-11-24)

### Features

- scrollback search and copy mode (DECK-24) ([3de8935](https://github.com/marcosmoura/deck/commit/3de89359ee8837674a911b72e946ccba1a94124f))
- show blinking cursor shapes (DECK-30) ([4d0d383](https://github.com/marcosmoura/deck/commit/4d0d383063e9117c734a1114f042dd8031fbf0f4))
- terminal compatibility (DECK-22) ([7bc365b](https://github.com/marcosmoura/deck/commit/7bc365bccb3376c7c74e8bc385f630087d8b53eb))

## 0.1.0 (2026-11-24)

### Features

- add proper config for Zed editor ([54ae8cd](https://github.com/marcosmoura/deck/commit/54ae8cd55e6a2d5be1572dd084323d1b0c0f2646))
- agent logos as images (DECK-12) ([e9d9e77](https://github.com/marcosmoura/deck/commit/e9d9e77fef2373519505c3232f9d79ac8456444e))
- agent status, hooks and resume (DECK-06) ([596d391](https://github.com/marcosmoura/deck/commit/596d3915c3d7ec11fca8417e2f41aad60278aeef))
- built-in icons (DECK-20) ([0f6f622](https://github.com/marcosmoura/deck/commit/0f6f622b8887cf191962d98fbfa7cccbaa8d5f4a))
- call projects spaces in the ui, config and cli (DECK-16) ([1207b33](https://github.com/marcosmoura/deck/commit/1207b3352e6a7ac62bec20ee6fdfe0075b14fb76))
- config and theming (DECK-08) ([c8863b7](https://github.com/marcosmoura/deck/commit/c8863b782b2466e3ddaf56b959447883d9a105f9))
- control API (DECK-05) ([e5285a1](https://github.com/marcosmoura/deck/commit/e5285a1a1bbc0de62488fc1202cf6fea6d601ca2))
- core daemon and terminal (DECK-01) ([becc81d](https://github.com/marcosmoura/deck/commit/becc81d2146b9785396e53864bd42d6c9b83edd1))
- daemon stats and stress tests (DECK-23) ([3d0f85c](https://github.com/marcosmoura/deck/commit/3d0f85ce063f2607c19f7e1ccd4a5e03b2a5eb79))
- **daemon:** report term_program as deck in panels ([b6e63cf](https://github.com/marcosmoura/deck/commit/b6e63cffc70b2567920b03522dd29c367ebd74d7))
- draw agent icons in their provider colour ([4085f55](https://github.com/marcosmoura/deck/commit/4085f550501c4714d475c9a5992e8f710d83044e))
- floating bar, sidebar highlight and context menus (DECK-17) ([9499544](https://github.com/marcosmoura/deck/commit/9499544f1299eb2d4b2874db3a0adf7afd52152f))
- grid layout (DECK-03) ([1948eae](https://github.com/marcosmoura/deck/commit/1948eae4d7d9bd0ad8966ab39ffc1d851abb66ed))
- images in panels (DECK-19) ([7f787ae](https://github.com/marcosmoura/deck/commit/7f787ae89772dcc5ce1e51abb643ca368a7e7134))
- log why the daemon stops ([30f1aea](https://github.com/marcosmoura/deck/commit/30f1aea69d540f361a225ea81c8958ca3bb26ccf))
- mouse text selection, copy on select (DECK-15) ([26f0ae9](https://github.com/marcosmoura/deck/commit/26f0ae92c775963a4c1c5d20a52e132f59fa6592))
- persistence across daemon restarts (DECK-04) ([941fcb2](https://github.com/marcosmoura/deck/commit/941fcb26a0a013cd9c9a141a03e87d00a61519c5))
- release automation and homebrew (DECK-25) ([105c35e](https://github.com/marcosmoura/deck/commit/105c35e83f3afd4511025355557656921b24a9bd))
- **repo:** add release workflows, install script and just install (DECK-14) ([bd5750e](https://github.com/marcosmoura/deck/commit/bd5750e8ed48e40cb3dbdd6868d0fd6e1463aeac))
- report opencode v2 status through a tui plugin (DECK-13) ([7f72dc5](https://github.com/marcosmoura/deck/commit/7f72dc5657b0d4c05f84bfd3a7b95f3cd2a282e8))
- restore agent sessions from a dialog (DECK-11) ([10def9f](https://github.com/marcosmoura/deck/commit/10def9f66bb1539eeab1a5760b782ed9f71cf71e))
- sessions, projects, panels and the sidebar (DECK-02) ([f9ba15e](https://github.com/marcosmoura/deck/commit/f9ba15ead54ed28972717a089fd5d5c36058fd10))
- sidebar mouse resize, new project button and new menu (DECK-10) ([017a9de](https://github.com/marcosmoura/deck/commit/017a9de80e1f37d8479158b27d08d662f67d3fcb))
- sidebar reorder and space gaps (DECK-18) ([f13dc19](https://github.com/marcosmoura/deck/commit/f13dc19e7467c6953d39e8031e74b04660710545))
- sidebar titles with program and agent icons (DECK-09) ([ba49fc9](https://github.com/marcosmoura/deck/commit/ba49fc9279867ae31770d121113b6e7cefd78668))
- terminals in the sidebar (DECK-27) ([bcad05e](https://github.com/marcosmoura/deck/commit/bcad05e98f44132ed2de166ee6538016e20f8f26))
- **tui:** flash a message when a selection is copied ([fabbdd9](https://github.com/marcosmoura/deck/commit/fabbdd99e1a51101c165a49add837d54613f23ba))

### Bug Fixes

- close the races that failed the suite on linux ci ([88b1d90](https://github.com/marcosmoura/deck/commit/88b1d90f6dda212d9e963c11b83591a69b077d13))
- **core:** use the claude and openai nerd font glyphs for agent icons ([9cca6f2](https://github.com/marcosmoura/deck/commit/9cca6f2aad12e5840b2f87907fc9924d42d9c79a))
- **daemon:** ignore a terminal's process before it runs its program ([fbcdaec](https://github.com/marcosmoura/deck/commit/fbcdaec0d1e0d75e5298130fb6174a5e4bf16bcb))
- keep the outer terminal's cursor shape when a program sets none ([0b9802d](https://github.com/marcosmoura/deck/commit/0b9802d75dd80b1363cdead368f14d74c72a5d55))
- **repo:** format every file release-please writes in the fix-up job ([dae2d23](https://github.com/marcosmoura/deck/commit/dae2d235c990aa4bd170580607857fe4b0945cb4))
- **repo:** install nextest from its cargo-nextest release tags ([533e928](https://github.com/marcosmoura/deck/commit/533e9288b1a2258977a11ae574aaf5e08fb96fe7))
- **repo:** isolate just run from an enclosing deck session ([97fff28](https://github.com/marcosmoura/deck/commit/97fff28084d1599275e401a6b23d2b77f4bfc8c3))
- **repo:** publish the formula through the shared tap action ([8649f1c](https://github.com/marcosmoura/deck/commit/8649f1c906f59db7d7218808e3895f39ad770a0a))
- **repo:** skip git-ignored files in oxfmt ([aa26ce4](https://github.com/marcosmoura/deck/commit/aa26ce48772d70ae041ce855f54ede262e11b543))
- **tui:** match sidebar clicks against the rows as drawn ([8e447b4](https://github.com/marcosmoura/deck/commit/8e447b42d65dc24fe79fc908602b7b4820708228))
- **tui:** pad the new space button and rename restore ([e7f2537](https://github.com/marcosmoura/deck/commit/e7f25379e5057c4a01f74b3699622f11493cfa72))
- **tui:** title the sidebar workspaces ([55f5069](https://github.com/marcosmoura/deck/commit/55f50693479c07c96db6fb01a04cf60ffdc5111b))
