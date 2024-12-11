<br />
<div align="center">
  <a href="https://github.com/losses/rune">
    <img src="https://github.com/Losses/rune/blob/master/assets/icons/breeze/apps/512/rune.png?raw=true" alt="Logo" width="80" height="80">
  </a>

  <h1 align="center">Rune</h1>

  <p align="center">
    The player that blends classic design with modern technology.
  </p>
  <p>
    <a href="https://github.com/Losses/rune/blob/master/LICENSE"><img alt="GitHub License" src="https://img.shields.io/github/license/losses/rune" /></a>
    <a href="https://github.com/Losses/rune/blob/master/flake.nix"><img alt="Nix Flakes" src="https://img.shields.io/badge/nix-flake-blue?logo=nixos" /></a>
    <a href="https://github.com/Losses/rune/issues"><img alt="GitHub Issues" src="https://img.shields.io/github/issues/losses/rune" /></a>
    <a href="https://github.com/Losses/rune/pulls"><img alt="GitHub Pull Requests" src="https://img.shields.io/github/issues-pr/losses/rune" /></a>
  </p>
</div>

https://github.com/user-attachments/assets/f363ff5e-d8e5-49b1-81f1-74efa831f83a

## Introduction

Rune Player is a music player that offers audio analysis and recommendation features. It introduces a new, modern music management paradigm to enhance your experience.

## Motivation

Rune is a UX experiment inspired by the question: What if Zune had continued to evolve? Using a modern tech stack, Rune captures the essence of Zune, drawing from the design highlights of various Windows products since the Windows XP era, including Windows Mobile and Windows Media Center.

The motivation behind Rune is to provide a modernized audio listening and management experience. With the rise of streaming services, localized music players have become less common, and many well-known media players have stagnated. Rune aims to offer a clean, consistent, and efficient audio playback experience.

## Unique Features

- **Distinctive Design**: Rune boasts a unique design style that respects user intent with no intrusive ads or unnecessary UI elements.
- **Highly Customizable**: A complex responsive system ensures optimal viewing on various screen sizes, with comprehensive keyboard support and customizable color themes.
- **Audio Analysis and Recommendations**: Users can analyze their entire music library to receive recommendations based on tracks, albums, or playlists.
- **Dynamic "Mix"**: Users can create dynamic playlists based on customizable rules, such as:

  - "Similar to this album"
  - "Top 100 most played tracks"
  - "Tracks similar to my favorites"
  - "Playlists including specific artists"

  These mixes automatically update as users listen to and add new tracks.

- **Cross-Platform Support**: Originally for Windows and NixOS, Rune now runs on macOS, general Linux distributions, and even Steam Deck. An Android version is under development.

## Pricing

**Rune is not a free pint of beer.** To use it legally, you should purchase a license. The price basement is $10 across all platforms, though it may vary due to store policies and regional differences.

However, we offer a WinRAR-style trial mechanism, allowing you to try the software indefinitely until satisfied. Due to its unique design philosophy, **we encourage thorough testing before purchase to ensure it suits your needs.**

Please note that we do not accept feature requests from unpaid users.

<p align="center">
  <a href="https://partner.steamgames.com/apps/landing/3343500"><img src="assets/available_steam.png" width=180 alt="Available on Steam"/></a>
  <a href="https://apps.microsoft.com/detail/9N52TW1F5348"><img src="assets/available_ms_store.png" width=180 alt="Available on Microsoft Store"/></a>
  <a href="#"><img src="assets/available_app_store.png" width=180 alt="Available on App Store"/></a>
</p>

## Installation and Running

Rune offers [CI builds](https://github.com/Losses/rune/actions) for Linux (including SteamOS), Windows, and macOS. You can find the latest versions on the Release page.

Alternatively, you can compile your own binary package. The [documentation](https://github.com/Losses/rune/blob/master/documents/compiling.md) and [CI scripts](https://github.com/Losses/rune/blob/master/.github/workflows/build.yml) serve as excellent references.

## Release Policy

Rune is divided into two release queues: the Production Queue and the Cutting Edge Queue. These queues release independently.

- **Production Queue**: Maintained by the Rune development team, ensuring no critical issues affect user experience before being pushed to stores. We only address software defects here, and no new features are introduced. Fixes go through Dev, Alpha, Beta, and RC stages before release.
- **Cutting Edge Queue**: Offers experimental features and is maintained by the community. Software packages from this queue may be available in community repositories, but Rune will never officially push these to users.

If a version in the Cutting Edge Queue reaches production quality after the RC stage, it will replace the version in the Production Queue and be pushed to users.

## Contributing

We're thrilled you're interested in contributing. Before you dive in, please take a moment to review these key points to ensure smooth collaboration. For detailed instructions, please refer to the full [Contributing Guide](CODE_OF_CONDUCT.md).

- **Language Requirement**: To facilitate clear communication across all developers, all contributions, including issues and pull requests, must be submitted in English. If you are not confident in your English proficiency, consider using a language model for assistance.
- **Feature Requests**: We do not accept feature requests from unpaid users.
- **Feature Implementation**: If you have a feature proposal, please reach out to the development team for a preliminary discussion to ensure it aligns with Rune's vision.
- **Documentation**: Collaboration is conducted exclusively in English. While we do not accept pull requests for translating the README or other development documentation, contributions for translating Rune itself are welcome.

## Acknowledgments

We extend our gratitude to the open-source project [Meyda](https://github.com/meyda/meyda) for enabling the audio analysis functionality. We also thank the countless developers in the Rust and Flutter ecosystems for making Rune possible.

## License

This project is licensed under the MPL License.

[^1]: All mentioned Microsoft products are trademarks of Microsoft. This project is not affiliated with Microsoft, and the founders of this project are not Microsoft employees.
