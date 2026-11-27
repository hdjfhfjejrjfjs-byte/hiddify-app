import 'dart:ffi' show Abi;
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hiddify/core/model/constants.dart';
import 'package:hiddify/core/model/environment.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:path/path.dart' as p;

/// The issues bot's link with this device's details as its start parameter: `;`-separated
/// `key=value` pairs, base64url without padding. Each key has one name and each value one code;
/// the bot has no aliases.
///
/// - `os`: `android` `ios` `win` `mac` `tv`, a Linux distro's code ([linuxDistro]) or `linux`
/// - `ov`: the OS or distro version, short (`14`, `11`, `24.04`)
/// - `v`: the build number, the part after `+` in pubspec's version
/// - `pm`: how the app was installed ([_packageMode])
/// - `t`: `c` crash, `b` bug
/// - `l`: the app's language; `a`: the CPU architecture
///
/// What does not fit Telegram's 64 characters is left out: `a`, then `l`, then `ov` is shortened.
@visibleForTesting
Uri telegramReportLink({
  required String os,
  required String ov,
  required String v,
  required String t,
  String? pm,
  String? l,
  String? a,
}) {
  final payload = hiddifyRust.require(
    hiddifyRust.telegramPayload(os: os, ov: ov, v: v, t: t, pm: pm, l: l, a: a),
    'telegramPayload',
  );
  return Uri.https('t.me', '/${Constants.bugReportTelegramBot}', {'start': payload});
}

/// The link for this device: [crash] after a crash, else a bug; [build] is the app's build
/// number, [language] its UI language, [release] what it was built as.
Future<Uri> telegramReportLinkFor({
  required bool crash,
  required String build,
  required String language,
  required bool tv,
  required Release release,
}) async {
  final (os, ov) = await _system(tv: tv);
  return telegramReportLink(
    os: os,
    ov: ov,
    v: build,
    pm: await _packageMode(release),
    t: crash ? 'c' : 'b',
    l: language,
    a: _arch(),
  );
}

/// The `os` code and its version: the release a person knows, not the build string.
Future<(String, String)> _system({required bool tv}) async {
  final full = Platform.operatingSystemVersion;
  String number() => RegExp(r'\d+(\.\d+){0,2}').firstMatch(full)?.group(0) ?? '';
  if (Platform.isAndroid) {
    var release = '';
    try {
      release = await const MethodChannel('com.hiddify.app/platform').invokeMethod<String>('os_version') ?? '';
    } on PlatformException {
      // the bot asks for it
    }
    return (tv ? 'tv' : 'android', release);
  }
  if (Platform.isIOS) return ('ios', number());
  if (Platform.isMacOS) return ('mac', number());
  if (Platform.isWindows) return ('win', windowsVersion(full));
  if (Platform.isLinux) {
    // inside a Flatpak, /etc is the runtime's: the host's own file is under /run/host
    for (final path in const ['/run/host/os-release', '/etc/os-release', '/usr/lib/os-release']) {
      try {
        final (os, ov) = linuxDistro(File(path).readAsStringSync());
        // a distro without a version (a rolling one) leaves the kernel's
        return (os, ov.isEmpty ? number() : ov);
      } on FileSystemException {
        continue;
      }
    }
    return ('linux', number());
  }
  return (Platform.operatingSystem, number());
}

/// `11` or `10`, from Windows's `"Windows 10 Pro" 10.0 (Build 22631)`: Windows 11 still says 10.0,
/// only its build tells.
@visibleForTesting
String windowsVersion(String operatingSystemVersion) =>
    hiddifyRust.require(hiddifyRust.windowsVersion(operatingSystemVersion), 'windowsVersion');

/// The bot's code for the distro an os-release file names, with its version; `linux` for one the
/// bot does not know.
@visibleForTesting
(String, String) linuxDistro(String osRelease) =>
    hiddifyRust.require(hiddifyRust.linuxDistro(osRelease), 'linuxDistro');

/// How this copy was installed, as the bot names it: the store's own build, or else what the
/// files around the app say; null when nothing tells.
Future<String?> _packageMode(Release release) async {
  bool exists(String path) => FileSystemEntity.typeSync(path) != FileSystemEntityType.notFound;

  if (Platform.isAndroid) return release == Release.googlePlay ? 'play' : 'apk';
  if (Platform.isIOS) return 'appstore';
  if (Platform.isMacOS) {
    if (release == Release.appStore) return 'appstore';
    // a Homebrew cask keeps its record in the Caskroom, on Apple silicon and on Intel
    if (exists('/opt/homebrew/Caskroom/hiddify') || exists('/usr/local/Caskroom/hiddify')) return 'brew';
    return 'dmg';
  }
  if (Platform.isWindows) {
    if (release == Release.microsoftStore) return 'msix';
    if (Platform.resolvedExecutable.toLowerCase().contains(r'\scoop\apps\')) return 'scoop';
    final programData = Platform.environment['ProgramData'] ?? r'C:\ProgramData';
    if (exists(p.join(programData, 'chocolatey', 'lib', 'hiddify'))) return 'choco';
    return Environment.isPortable ? 'portable' : 'exe';
  }
  if (Platform.isLinux) {
    final environment = Platform.environment;
    if ((environment['APPIMAGE'] ?? '').isNotEmpty) return 'appimage';
    if ((environment['FLATPAK_ID'] ?? '').isNotEmpty) return 'flatpak';
    if ((environment['SNAP'] ?? '').isNotEmpty) return 'snap';
    // each package manager's record of what it installed
    if (exists('/var/lib/dpkg/info/hiddify.list')) return 'deb';
    try {
      final pacman = Directory('/var/lib/pacman/local');
      if (pacman.existsSync() && pacman.listSync().any((e) => p.basename(e.path).startsWith('hiddify'))) {
        return 'aur';
      }
    } on FileSystemException {
      // not readable: not this one
    }
    try {
      final rpm = await Process.run('rpm', ['-q', '--quiet', 'hiddify']);
      if (rpm.exitCode == 0) return 'rpm';
    } on ProcessException {
      // no rpm on this system
    }
    return null;
  }
  return null;
}

/// `arm64`, `x64`, `arm`, `ia32` or `riscv64`.
String _arch() => Abi.current().toString().split('_').last;
