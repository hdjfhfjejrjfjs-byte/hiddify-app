// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/utils/link_parsers.dart
// Replaced by:   rust/crates/hiddify-profile (links, base64)

import 'dart:convert';

import 'validators.dart';

typedef ProfileLink = ({String url, String name});

abstract class LegacyLinkParser {
  static String generateSubShareLink(String url, [String? name]) {
    final uri = Uri.tryParse(url);
    if (uri == null) return '';
    return uri.replace(fragment: name).toString();
  }

  static const protocols = ['hiddify', 'v2ray', 'v2rayn', 'v2rayng', 'clash', 'clashmeta', 'sing-box'];

  static ProfileLink? parse(String link) => simple(link) ?? deep(link);

  static ProfileLink? simple(String link) {
    if (!isUrl(link)) return null;
    final uri = Uri.parse(link.trim());
    return (url: uri.toString(), name: uri.queryParameters['name'] ?? '');
  }

  static ProfileLink? deep(String link) {
    final uri = Uri.tryParse(link.trim());
    if (uri == null || !uri.hasScheme || !uri.hasAuthority) return null;
    final queryParams = uri.queryParameters;
    switch (uri.scheme) {
      case 'hiddify':
        if (queryParams.containsKey('url')) {
          final name = (queryParams['name']?.isNotEmpty ?? false) ? queryParams['name']! : uri.fragment;
          return (url: queryParams['url']!, name: name);
        } else {
          return (url: uri.path.substring(1) + (uri.hasQuery ? "?${uri.query}" : ""), name: uri.fragment);
        }
      case 'v2ray' || 'v2rayn' || 'v2rayng' || 'clash' || 'clashmeta' || 'sing-box':
        return queryParams.containsKey('url') ? (url: queryParams['url']!, name: queryParams['name'] ?? '') : null;
      default:
        return null;
    }
  }
}

String legacySafeDecodeBase64(String str) {
  try {
    return utf8.decode(base64Decode(str));
  } catch (e) {
    return str;
  }
}
