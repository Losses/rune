<br />
<div align="center">
  <a href="https://github.com/othneildrew/Best-README-Template">
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

## Introduction

Rune Player is a music player that offers audio analysis and recommendation features. It introduces a new, modern music management paradigm to enhance your experience.

## Motivation

Rune is a UX experiment inspired by the question: What if Zune[^1] had continued to evolve? Using a modern tech stack (Flutter + Rust), Rune captures the essence of Zune, drawing from the design highlights of various Windows products since the Windows XP era, including Windows Mobile and Windows Media Center[^1].

The motivation behind Rune is to provide a modernized audio listening and management experience. With the rise of streaming services, localized music players have become less common, and many well-known media players have stagnated. Rune aims to offer a clean, consistent, and efficient audio playback experience.

## Unique Features

- **Distinctive Design**: Rune boasts a unique design style.
- **Audio Analysis and Recommendations**: Users can analyze their entire music library to receive recommendations based on tracks, albums, or playlists.
- **Dynamic "Mix" Feature**: Users can create dynamic playlists based on customizable rules, such as:

  - "Similar to this album"
  - "Top 100 most played tracks"
  - "Tracks similar to my favorites"
  - "Playlists including specific artists"

  These mixes automatically update as users listen to and add new tracks.

## Installation and Running

Rune is still in the early stages of development and does not provide pre-compiled binaries. However, you can set up a development environment by following the [compiling instructions](documents/compiling.md).

## Contributing

We're thrilled you're interested in contributing. Before you dive in, please take a moment to review these key points to ensure smooth collaboration. For detailed instructions, please refer to the full [Contributing Guide](CONTRIBUTING.md).

- **Language Requirement**: To facilitate clear communication across all developers, all contributions, including issues and pull requests, must be submitted in English. If you are not confident in your English proficiency, consider using a language model for assistance.
- **Feature Requests**: As Rune is in the early stages of development, we are currently not accepting feature requests.
- **Feature Implementation**: If you have a feature proposal, please reach out to the development team for a preliminary discussion to ensure it aligns with Rune's vision.
- **Documentation**: Collaboration is conducted exclusively in English. While we do not accept pull requests for translating the README or other development documentation, contributions for translating Rune itself are welcome.

## Acknowledgments

We extend our gratitude to the open-source project [Meyda](https://github.com/meyda/meyda) for enabling the audio analysis functionality. We also thank the countless developers in the Rust and Flutter ecosystems for making Rune possible.

## License

This project is licensed under the MPL License.

[^1]: All mentioned Microsoft products are trademarks of Microsoft. This project is not affiliated with Microsoft, and the founders of this project are not Microsoft employees.
