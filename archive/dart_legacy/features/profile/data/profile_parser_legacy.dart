// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/profile/data/profile_parser.dart (parsing & override-merge methods)
// Replaced by:   rust/crates/hiddify-profile & rust/crates/hiddify-config

import 'dart:convert';

import 'package:hiddify/core/model/optional_range.dart';
import 'package:hiddify/features/profile/data/profile_parser.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/singbox/model/singbox_config_enum.dart';
import 'package:hiddify/singbox/model/singbox_proxy_type.dart';

abstract class LegacyProfileParser {
  static bool isSubscriptionLink(String line) {
    if (!line.startsWith('http://') && !line.startsWith('https://')) return false;
    final path = Uri.tryParse(line.trimRight())?.path ?? '';
    return path.isNotEmpty && path != '/';
  }

  static Map<String, dynamic> populateHeaders({
    required String content,
    Map<String, dynamic>? remoteHeaders,
  }) {
    final contentHeaders = _parseHeadersFromContent(content);
    return _mergeAndValidateHeaders(contentHeaders, remoteHeaders ?? {});
  }

  static Map<String, dynamic> _mergeAndValidateHeaders(
    Map<String, dynamic> contentHeaders,
    Map<String, dynamic> remoteHeaders,
  ) {
    for (final entry in contentHeaders.entries) {
      if (!remoteHeaders.keys.contains(entry.key)) {
        remoteHeaders[entry.key] = entry.value;
      }
    }
    final headers = <String, dynamic>{};
    for (final entry in remoteHeaders.entries) {
      if (ProfileParser.allowedProfileHeaders.contains(entry.key) &&
          entry.value != null &&
          entry.value.toString().isNotEmpty) {
        headers[entry.key] = entry.value;
      }
    }
    return headers;
  }

  static Map<String, dynamic> _parseHeadersFromContent(String content) {
    final headers = <String, dynamic>{};
    String content_;
    try {
      content_ = utf8.decode(base64Decode(content));
    } catch (_) {
      content_ = content;
    }
    final lines = content_.split("\n");
    final linesToProcess = lines.length < 10 ? lines.length : 10;
    for (int i = 0; i < linesToProcess; i++) {
      final line = lines[i];
      if (line.startsWith("#") || line.startsWith("//")) {
        final index = line.indexOf(':');
        if (index == -1) continue;
        final key = line.substring(0, index).replaceFirst(RegExp("^#|//"), "").trim().toLowerCase();
        final value = line.substring(index + 1).trim();
        headers[key] = value;
      }
    }
    return headers;
  }

  static SubscriptionInfo? parseSubscriptionInfo(String subInfoStr) {
    final map = <String, int?>{};
    for (final segment in subInfoStr.split(';')) {
      final parts = segment.split('=');
      if (parts.length < 2) continue;
      final key = parts.first.trim();
      if (key.isEmpty) continue;
      final value = num.tryParse(parts[1].trim());
      map[key] = (value == null || !value.isFinite || value < 0) ? null : value.toInt();
    }
    if (map case {"upload": final upload?, "download": final download?, "total": final total, "expire": var expire}) {
      final total1 = (total == null || total == 0) ? ProfileParser.infiniteTrafficThreshold + 1 : total;
      expire = (expire == null || expire == 0 || expire > ProfileParser.infiniteTimeThreshold)
          ? ProfileParser.infiniteTimeThreshold
          : expire;
      return SubscriptionInfo(
        upload: upload,
        download: download,
        total: total1,
        expire: DateTime.fromMillisecondsSinceEpoch(expire * 1000),
      );
    }
    return null;
  }

  static String filenameFromContentDisposition(String header) {
    if (RegExp(r"filename\*\s*=\s*(?:[^']*'[^']*')?([^;]+)", caseSensitive: false).firstMatch(header) case final m?) {
      final raw = m.group(1)?.trim() ?? '';
      if (raw.isNotEmpty) {
        try {
          return Uri.decodeComponent(raw);
        } catch (_) {}
      }
    }
    if (RegExp('filename="([^"]*)"').firstMatch(header) case final m?) {
      return m.group(1) ?? '';
    }
    return '';
  }

  static String decodeBase64Title(String value) {
    try {
      return utf8.decode(base64.decode(base64.normalize(value.trim()))).trim();
    } on FormatException {
      return '';
    }
  }

  static String protocol(String content) {
    if (content.contains("[Interface]")) {
      return ProxyType.wireguard.label;
    }
    if (content.contains('<ca>') && RegExp(r'^\s*remote\s', multiLine: true).hasMatch(content)) {
      return 'OpenVPN';
    }
    final lines = content.split('\n');
    String? name;
    for (final line in lines) {
      final uri = Uri.tryParse(line);
      if (uri == null) continue;
      final fragment = uri.hasFragment ? Uri.decodeComponent(uri.fragment.split(" -> ")[0]) : null;
      name ??= switch (uri.scheme) {
        'ss' => fragment ?? ProxyType.shadowsocks.label,
        'ssconf' => fragment ?? ProxyType.shadowsocks.label,
        'vmess' => ProxyType.vmess.label,
        'vless' => fragment ?? ProxyType.vless.label,
        'trojan' => fragment ?? ProxyType.trojan.label,
        'tuic' => fragment ?? ProxyType.tuic.label,
        'hy2' || 'hysteria2' => fragment ?? ProxyType.hysteria2.label,
        'hy' || 'hysteria' => fragment ?? ProxyType.hysteria.label,
        'ssh' => fragment ?? ProxyType.ssh.label,
        'wg' => fragment ?? ProxyType.wireguard.label,
        'awg' => fragment ?? ProxyType.awg.label,
        'shadowtls' => fragment ?? ProxyType.shadowtls.label,
        'mieru' => fragment ?? ProxyType.mieru.label,
        'warp' => fragment ?? ProxyType.warp.label,
        'ovpn' => fragment ?? 'OpenVPN',
        final scheme when scheme.isNotEmpty && line.trimLeft().startsWith('$scheme://') =>
          fragment ?? _schemeLabel(scheme),
        _ => null,
      };
    }
    return name ?? ProxyType.unknown.label;
  }

  static String _schemeLabel(String scheme) {
    final base = scheme.split('+').first;
    final type = ProxyType.fromJson(base);
    if (type != ProxyType.unknown) return type.label;
    return base[0].toUpperCase() + base.substring(1);
  }

  static String profileOverride({
    required Map<String, dynamic>? populatedHeaders,
    required UserOverride? userOverride,
  }) {
    final headers = Map<String, dynamic>.from(populatedHeaders ?? {});

    final fragmentInput = headers.remove('fragment')?.toString();
    final extraSecurityInput = headers.remove('extra-security')?.toString();
    final keepTopInput = headers.remove('keep-top')?.toString();
    headers.remove('keep-top-quick');
    headers.remove('keep-top-countries');
    headers.remove('keep-top-pinned');

    final mode =
        ProfileParser.extraSecurityMode(userOverride?.extraSecurity) ??
        ProfileParser.extraSecurityMode(extraSecurityInput);
    if (mode != null) {
      headers['chain-status'] = ChainStatus.extraSecurity.key;
      headers['extra-security'] = {'mode': mode.key};
    }

    if (userOverride?.enableFragment == true) {
      headers['tls-tricks'] = {'enable-fragment': true};
    } else if (ProfileParser.fragmentTricks(fragmentInput) case final tricks?) {
      headers['tls-tricks'] = tricks;
    }

    headers['keep-top'] = userOverride?.keepTop ?? ProfileParser.keepTopHeader(keepTopInput) ?? ProfileParser.keepTopDefault;
    if (userOverride?.keepTopQuick ?? true) {
      headers['keep-top-quick'] = true;
    }
    if (userOverride?.keepTopCountries case final countries? when countries.isNotEmpty) {
      headers['keep-top-countries'] = countries;
    }
    if (userOverride?.keepTopPinned case final pinned? when pinned.isNotEmpty) {
      headers['keep-top-pinned'] = pinned;
    }
    if (userOverride?.keepTags case final tags? when tags.isNotEmpty) {
      headers['keep-tags'] = tags;
    }

    headers.removeWhere(
      (key, value) => !ProfileParser.allowedOverrideConfigs.contains(key) || value == null || value.toString().isEmpty,
    );

    return jsonEncode({for (final key in headers.keys) key: headers[key]});
  }

  static Map<String, dynamic> applyProfileOverride(Map<String, dynamic> main, String? profileOverride) {
    if (profileOverride == null) return main;
    if (profileOverride.contains("{")) {
      final profileOverrideMap = jsonDecode(profileOverride) as Map<String, dynamic>;
      return _mergeJson(main, profileOverrideMap);
    } else {
      return main;
    }
  }

  static Map<String, dynamic> _mergeJson(Map<String, dynamic> main, Map<String, dynamic> override) {
    override.forEach((key, value) {
      if (main.containsKey(key)) {
        if (main[key] is Map<String, dynamic> && value is Map<String, dynamic>) {
          main[key] = _mergeJson(main[key] as Map<String, dynamic>, value);
        } else {
          main[key] = value;
        }
      } else {
        main[key] = value;
      }
    });
    return main;
  }
}
