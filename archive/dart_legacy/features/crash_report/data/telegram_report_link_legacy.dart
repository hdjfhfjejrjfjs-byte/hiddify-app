// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/crash_report/data/telegram_report_link.dart (payload encoding, windowsVersion, linuxDistro)
// Replaced by:   rust/crates/hiddify-diagnostics (telegram_payload, windows_version, linux_distro)

import 'dart:convert';

const _maxPayload = 64;

String legacyTelegramPayload({
  required String os,
  required String ov,
  required String v,
  required String t,
  String? pm,
  String? l,
  String? a,
}) {
  String encode(Map<String, String?> fields) => base64Url
      .encode(
        utf8.encode(
          [
            for (final MapEntry(:key, :value) in fields.entries)
              if (value != null && _clean(value).isNotEmpty) '$key=${_clean(value)}',
          ].join(';'),
        ),
      )
      .replaceAll('=', '');

  var version = ov;
  Map<String, String?> fields({bool language = true, bool arch = true}) => {
    'os': os,
    'ov': version,
    'v': v,
    'pm': pm,
    't': t,
    if (language) 'l': l,
    if (arch) 'a': a,
  };
  var payload = encode(fields());
  if (payload.length > _maxPayload) payload = encode(fields(arch: false));
  if (payload.length > _maxPayload) payload = encode(fields(arch: false, language: false));
  while (payload.length > _maxPayload && version.isNotEmpty) {
    version = version.substring(0, version.length - 1);
    payload = encode(fields(arch: false, language: false));
  }
  return payload;
}

String legacyWindowsVersion(String operatingSystemVersion) {
  final build = int.tryParse(RegExp(r'Build (\d+)').firstMatch(operatingSystemVersion)?.group(1) ?? '');
  if (build == null) return '';
  return build >= 22000 ? '11' : '10';
}

(String, String) legacyLinuxDistro(String osRelease) {
  final values = {
    for (final line in const LineSplitter().convert(osRelease))
      if (line.indexOf('=') case final at when at > 0)
        line.substring(0, at).trim(): line.substring(at + 1).trim().replaceAll(RegExp(r'''^["']|["']$'''), ''),
  };
  final id = (values['ID'] ?? '').toLowerCase();
  final code = switch (id) {
    'ubuntu' => 'ubu',
    'debian' => 'deb',
    'fedora' => 'fed',
    'linuxmint' => 'mint',
    'arch' => 'arch',
    'manjaro' => 'manj',
    'endeavouros' => 'eos',
    'cachyos' => 'cachy',
    'garuda' => 'garu',
    'pop' => 'pop',
    'elementary' => 'elem',
    'zorin' => 'zorin',
    'mx' => 'mx',
    'neon' => 'neon',
    'kali' => 'kali',
    'almalinux' => 'alma',
    'rocky' => 'rocky',
    'centos' => 'cent',
    'rhel' => 'rhel',
    'nixos' => 'nix',
    'gentoo' => 'gen',
    'void' => 'void',
    'alpine' => 'alp',
    'deepin' => 'deep',
    'kylin' || 'ubuntukylin' => 'kylin',
    'astra' => 'astra',
    'solus' => 'sol',
    'slackware' => 'slack',
    'steamos' => 'steam',
    'bazzite' => 'bazz',
    _ when id.startsWith('opensuse') || id == 'sles' => 'sus',
    _ => 'linux',
  };
  return (code, values['VERSION_ID'] ?? '');
}

String _clean(String value) => value.replaceAll(RegExp('[^A-Za-z0-9._-]'), '');
