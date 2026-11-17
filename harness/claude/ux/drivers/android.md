# Driver: Android apps

For screens whose `interface` is `mobile` and whose platform is Android. Command-line tools only.

## Tools needed

- `adb` (`adb version`)
- `emulator` (the Android emulator command line) and `avdmanager` or `emulator -list-avds`
- `uiautomator` is used through `adb shell`, so it needs no extra tool

When one is missing, run `bf evidence unreachable <check> --reason "<tool> missing for Android screen <S-id>"`. Never pass.

## Start the product

1. Pick an AVD: `emulator -list-avds`. Start it in the background without a window: `emulator -avd <name> -no-window -no-audio`. Wait with `adb wait-for-device`, then until `adb shell getprop sys.boot_completed` prints `1`, with a fixed number of tries.
2. Install the built package: `adb install -r <apk>`.
3. Launch: `adb shell am start -n <package>/<activity>`.

## Act

- Tap: `adb shell input tap <x> <y>`; type: `adb shell input text '<text>'`; keys: `adb shell input keyevent <code>`; swipe: `adb shell input swipe <x1> <y1> <x2> <y2>`.
- Find coordinates from the screen's element tree: `adb shell uiautomator dump /sdcard/ui.xml`, then `adb pull /sdcard/ui.xml <scratch>/ui.xml`.

## Capture evidence

- `adb exec-out screencap -p > <scratch>/<id>.png`
- The element tree `ui.xml` for the accessibility result (content descriptions, text, clickable).
- `adb logcat -d` for the app's output.
- Attach one file per step with the step evidence command.

## Stop

`adb shell am force-stop <package>`, and `adb emu kill` for an emulator you started. Leave a device you did not start.
