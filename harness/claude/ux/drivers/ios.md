# Driver: iOS apps

For screens whose `interface` is `mobile` and whose platform is iOS. Command-line tools only. macOS only.

## Tools needed

- `xcrun` with `simctl` (`xcrun simctl help`)
- `xcodebuild` (to build the app, when the brief gives a build command)

When one is missing, or the host is not macOS, run `bf evidence unreachable <check> --reason "<tool> missing for iOS screen <S-id>"`. Never pass.

## Start the product

1. Pick an available simulator: `xcrun simctl list devices available`. Boot it: `xcrun simctl boot <udid>`, then `xcrun simctl bootstatus <udid>`.
2. Build as the brief says (`xcodebuild ... -derivedDataPath <scratch>/build`), then install: `xcrun simctl install <udid> <app path>`.
3. Launch: `xcrun simctl launch <udid> <bundle id>`.

## Act

`simctl` can open links (`xcrun simctl openurl <udid> <url>`), set the appearance (`xcrun simctl ui <udid> appearance dark`) and send a push. For taps and typing, use the launch arguments or the deep links the brief names. A step that needs a tap and has no command-line way to make it is unreachable; say so.

## Capture evidence

- `xcrun simctl io <udid> screenshot <scratch>/<id>.png`
- `xcrun simctl spawn <udid> log show --last 1m` or `xcrun simctl launch --console <udid> <bundle id>` for the app's output.
- Attach one file per step with the step evidence command.

## Stop

`xcrun simctl terminate <udid> <bundle id>`, and shut down the simulator you booted: `xcrun simctl shutdown <udid>`. Leave a simulator you did not boot.
