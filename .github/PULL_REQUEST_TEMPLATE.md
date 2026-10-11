<!-- Thanks for contributing! See CONTRIBUTING.md. -->

## What and why

<!-- What does this change, and why? Link the issue: "Fixes #123". -->

## How it was tested

<!-- Commands you ran and the OS/tablet you tried it on. -->

- [ ] Host: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (in `host/`)
- [ ] Desktop app: `npm run check`, `npx playwright test` (in `host/app/`)
- [ ] Android: `./gradlew testDebugUnitTest assembleDebug`
- [ ] Protocol: `python protocol/tools/validate.py protocol`
- [ ] Plugins: `python3 plugins/test_all.py`
- [ ] Not applicable (docs only)

## Checklist

- [ ] Every commit is signed off (`git commit -s`, see CONTRIBUTING.md).
- [ ] No code copied from GPL/AGPL/LGPL projects; new dependencies are permissively licensed and `scripts/third-party-notices.py` was re-run.
- [ ] No telemetry, Play Services or proprietary SDKs; anything online is opt-in.
- [ ] Protocol changes have schema and fixture updates and keep older clients working.
- [ ] Host networking, pairing or action changes went through the [security checklist](../docs/SECURITY.md).
- [ ] User-visible changes are reflected in `website/guide/` or the docs.
- [ ] No personal names, emails, tokens or home paths in code, tests, logs or screenshots.
