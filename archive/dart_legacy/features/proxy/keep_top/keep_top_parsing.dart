// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/proxy/keep_top/keep_top.dart (countProxies & profileProxies)
// Replaced by:   rust/crates/hiddify-config (count_proxies, profile_proxies)

import 'dart:convert';

typedef ProfileProxy = ({String tag, String type});

const _notProxies = {'block', 'selector', 'urltest', 'custom', 'dns'};
const _builtInTags = {'direct', 'bypass', 'block', 'select', 'lowest'};

int legacyCountProxies(String config) => legacyProfileProxies(config).length;

List<ProfileProxy> legacyProfileProxies(String config) {
  final json = jsonDecode(config);
  if (json is! Map) return const [];
  final proxies = <ProfileProxy>[];
  for (final key in const ['outbounds', 'endpoints']) {
    for (final out in json[key] as List? ?? const []) {
      if (out is! Map) continue;
      final tag = out['tag'] as String? ?? '';
      final type = out['type'] as String? ?? '';
      if (_notProxies.contains(type) || _builtInTags.contains(tag) || tag.contains('§hide§')) continue;
      proxies.add((tag: tag, type: type));
    }
  }
  return proxies;
}
