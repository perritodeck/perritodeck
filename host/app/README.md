# PerritoDeck desktop app

The host with a window: it sits in the tray, pairs tablets and shows your buttons.
Closing the window keeps the host running; quit from the tray icon.

Built with [Tauri 2](https://tauri.app) and Svelte 5. The Rust side is a thin layer over
the `perritodeck-host` library, which the command-line host uses too.

## Working on it

```sh
npm ci
npm run dev            # UI only, in a browser, against a mock host (src/lib/mock.ts)
npx tauri dev          # the real app, with the real host
npx playwright test    # UI tests against the mock
npm run check          # type-check
npx tauri build        # installers in src-tauri/target/release/bundle/
```

Linux needs the WebKitGTK and tray libraries:

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxkbcommon-dev
```

`src-tauri` is its own Cargo workspace so the headless host builds without them.
`PERRITODECK_DATA_DIR` and `PERRITODECK_PROFILE` work as they do for the command line.
Only one host can run at a time: quit `perritodeck-host` before starting the app.

Fonts: Bricolage Grotesque and Atkinson Hyperlegible, both SIL Open Font License 1.1,
bundled via Fontsource.
