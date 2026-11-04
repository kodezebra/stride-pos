# Stride Scan

The Android companion for Stride POS. It pairs with one till over the shop's
Wi-Fi and sends a barcode to it: no account, no cloud, and no second catalogue.

The main till is in [`../pos`](../pos). Read the top-level
[README](../../README.md) for what the till does.

## What it does

1. Open **Settings → Scanner** on the till, turn the listener on, and leave the
   pairing card visible.
2. On the phone, scan the QR code. If the camera cannot read it, type the
   address, port and code shown under it (ten characters, e.g. `7QK4M-2X9AB`;
   dashes, spaces and lower case are all accepted).
3. Choose what a scan should do:
   - **Sell — add to cart** sends `action: "sell"`, and the item lands in the
     till's cart.
   - **Add — open product form** sends `action: "add"`, and the till opens the
     product editor with the barcode prefilled.
4. Point the camera at a barcode.

A phone can pair with any till that shows a code; one till can take scans from
as many phones as the shop pairs. The last pairing is remembered, so a phone
that comes back to the same Wi-Fi opens straight into scan mode.

The camera preview and decoding stay on the phone. ML Kit's barcode model is
bundled in the APK, so scanning does not need an internet connection.

## Build

Needs a JDK with `javac` (17 or newer) and the Android SDK with platform 36 and
build-tools 36.0.0. Point `ANDROID_HOME` (or `ANDROID_SDK_ROOT`) at the SDK.

```bash
cd apps/scanner
./gradlew :app:assembleDebug
```

The debug APK is at `app/build/outputs/apk/debug/app-debug.apk`:

```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

A local machine whose `java` is a JRE without `javac` cannot run the Android
build. Point `JAVA_HOME` at a full JDK, for example the one bundled with Android
Studio:

```bash
JAVA_HOME="$HOME/Applications/android-studio/jbr" ./gradlew :app:assembleDebug
```

## Release

```bash
STRIDE_SCAN_VERSION=0.1.0 \
STRIDE_SCAN_VERSION_CODE=100 \
./gradlew :app:assembleRelease
```

The APK is at `app/build/outputs/apk/release/app-release.apk`.

A release is signed with the debug key unless all four signing variables are
present:

| Variable | Meaning |
|---|---|
| `STRIDE_SCAN_KEYSTORE` | path to the `.jks` keystore |
| `STRIDE_SCAN_STORE_PASSWORD` | keystore password |
| `STRIDE_SCAN_KEY_ALIAS` | key alias |
| `STRIDE_SCAN_KEY_PASSWORD` | key password |

The debug key keeps a sideloaded build installable, but it is not a stable
identity: a later build signed with a different key cannot update it. For a shop
that will keep the app, make one keystore, keep it safe, and use it for every
release.

The GitHub release workflow builds the APK from a `pos-v*` tag. It reads the
keystore from the `STRIDE_SCAN_KEYSTORE_BASE64` secret and the three passwords
and alias from `STRIDE_SCAN_STORE_PASSWORD`, `STRIDE_SCAN_KEY_ALIAS`, and
`STRIDE_SCAN_KEY_PASSWORD`. With no secrets it falls back to the debug key.

## Security and network limits

- The till listener is off until someone turns it on in Settings.
- Every scan carries the till's pairing token (`X-Till-Token`). A request without
  it is rejected before the body is read.
- The phone talks cleartext HTTP to the till. That is deliberate for a same-LAN
  tool: there is no certificate authority on a shop network, and the traffic
  never leaves it.
- Do not expose the till's port to the internet, and do not use the app over a
  public Wi-Fi network. The pairing code is visible on the till's Settings screen and in
  the QR code.
- A scan is dropped if the till is locked, setting up its currency, taking a
  payment, asking for approval, or showing any editor or modal. The phone may
  still say the scan was sent, because the listener accepted the request; the
  till is the thing that decides whether the cart can change. The cart is never
  changed off-screen.
- The phone ignores the same barcode for 1.5 seconds so one barcode held in front
  of the camera does not fill the cart.

## Troubleshooting

**"Could not reach the till."** Check that the phone and till are on the same
Wi-Fi, that the listener says *Listening* in Settings, and that the shop network
does not isolate clients (guest Wi-Fi often does).

A host firewall is the most common cause, because the till still answers on
`127.0.0.1` while every other machine is dropped. Settings → Scanner says which
firewall is in the way, if any. Press **Allow port**: the OS asks for the
administrator password once, and only that one TCP port is opened. If there is
no password dialog (a headless machine, or no polkit agent), the card prints the
command to run instead:

```sh
sudo firewall-cmd --add-port=47831/tcp --permanent   # Fedora, RHEL, openSUSE
sudo firewall-cmd --reload
sudo ufw allow 47831/tcp                             # Ubuntu, Debian
```

**The QR code will not scan.** Type the address, port and code instead. The
address is the till's LAN address, not `127.0.0.1`.

**The scan reaches the till but nothing happens.** Look at the till: it may be
locked, in a payment, or showing an editor. Close that screen and scan again.
The phone can still report success in that case, because the listener accepted
the request but the till dropped it before the cart. `Add` also needs a manager
or owner, because it changes the catalogue.

**The camera is black.** Grant the camera permission in Android settings and
reopen the app. Only one app can hold the camera at a time.
