## What and why

<!-- One topic per pull request. Link the issue it fixes, if any. -->

## How it was tested

<!-- Simulators, devices, server hardware, media used. -->

## Checklist

- [ ] `scripts/test-all.sh` (soon `make check`) passes locally. GitHub Actions doesn't run on pull
      requests, so this is the check.
- [ ] The app builds for iOS, tvOS and macOS (if app code changed).
- [ ] A bug fix comes with a test that fails without the fix.
- [ ] A refactor changes no behaviour (say how you checked).
- [ ] New UI checked on every platform it appears on (iPhone, iPad, Mac, Apple TV, Android TV).
- [ ] No personal details in the code, docs, tests, commit messages or screenshots: names, email
      addresses, IP addresses or host names, device names, tokens or keys.
- [ ] CHANGELOG.md has a line under "Unreleased" if a user would notice the change.
