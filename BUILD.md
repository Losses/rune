# Building Rune

## macOS

> This chapter provides instructions for building Rune on macOS for **development**. If you want to fork Rune and build your own version for production, you need to set up your own code signing, provisioning profiles, etc., which is not covered in this chapter.

### Prerequisites

- Xcode
- [Homebrew](https://brew.sh)

### Steps

1. Clone the repository:
2. Install all development dependencies:
   ```sh
   ./scripts/macos_1_install.sh
   ```

> If you're an employee of *Inkwire Tech*, make sure you have an Apple Account in *Inkwire Tech*'s Developer Program logged in on your Xcode, and skip to Step #6. Ask @laosb if you can't make it work.

3. Open the project in Xcode:
   ```sh
   open ./macos/Runner.xcworkspace
   ```
4. In Xcode, select the `Runner` project in the project navigator, then select the `Runner` target.
5. In the *Signing & Capabilities* tab:
    1. Uncheck *Automatically manage signing*.
    2. Select *None* for *Provisioning Profile*.
    3. Select *None* for *Team*.
    4. Select *Sign to Run Locally* for *Signing Certificate*.
6. Build / run the project:
    ```sh
    ./scripts/macos_2_build.sh
    # or
    ./scripts/macos_2_run.sh
    ```

We use the signing configuration in our production GitHub Actions workflow, so please don't commit and push any changes to the signing configuration.
