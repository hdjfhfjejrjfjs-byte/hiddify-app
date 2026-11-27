// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/proxy/data/single_proxy_config.dart
// Replaced by:   rust/crates/hiddify-config (single_proxy_config)

import 'dart:convert';

String? legacySingleProxyConfig(String tag, {String host = '', int port = 0, String? raw, String? generated}) {
  bool portAgrees(Map<String, dynamic> item) => port == 0 || item['server_port'] == null || item['server_port'] == port;
  bool hostAgrees(Map<String, dynamic> item) => host.isEmpty || item['server'] == null || item['server'] == host;

  if (_decode(raw) case final config?) {
    final name = _baseTag(tag);
    var matches = _items(config).where((item) => _baseTag('${item['tag']}') == name).toList();
    if (matches.length > 1) matches = matches.where(portAgrees).toList();
    if (matches.length > 1) matches = matches.where(hostAgrees).toList();
    if (matches.length == 1) return _extract(config, matches.single);
  }
  if (_decode(generated) case final config?) {
    final match = _items(config).where((item) => item['tag'] == tag).firstOrNull;
    if (match != null && portAgrees(match)) return _extract(config, match);
  }
  return null;
}

String _baseTag(String tag) => tag.split(' § ').first.trim();
bool _isHiddenTag(String tag) => tag.contains('§hide§');

const _sections = ['outbounds', 'endpoints'];

Map<String, dynamic>? _decode(String? content) {
  if (content == null || content.isEmpty) return null;
  try {
    final decoded = jsonDecode(content);
    return decoded is Map<String, dynamic> ? decoded : null;
  } catch (_) {
    return null;
  }
}

Iterable<Map<String, dynamic>> _items(Map<String, dynamic> config) sync* {
  for (final section in _sections) {
    if (config[section] case final List<dynamic> list) yield* list.whereType<Map<String, dynamic>>();
  }
}

String _extract(Map<String, dynamic> config, Map<String, dynamic> proxy) {
  final byTag = {for (final item in _items(config)) '${item['tag']}': item};

  final picked = <String, Map<String, dynamic>>{};
  Map<String, dynamic>? next = proxy;
  while (next != null && !picked.containsKey('${next['tag']}')) {
    final item = Map<String, dynamic>.of(next)..remove('domain_resolver');
    picked['${next['tag']}'] = item;
    final detour = item['detour'];
    next = detour is String && !_isHiddenTag(detour) ? byTag[detour] : null;
    if (next == null) item.remove('detour');
  }

  final endpointTags = {
    if (config['endpoints'] case final List<dynamic> list)
      for (final item in list.whereType<Map<String, dynamic>>()) '${item['tag']}',
  };
  final outbounds = <Map<String, dynamic>>[];
  final endpoints = <Map<String, dynamic>>[];
  for (final MapEntry(key: tag, value: item) in picked.entries) {
    (endpointTags.contains(tag) ? endpoints : outbounds).add(item);
  }
  picked.values.first['tag'] = _baseTag(picked.keys.first);
  return const JsonEncoder.withIndent(
    '  ',
  ).convert({if (outbounds.isNotEmpty) 'outbounds': outbounds, if (endpoints.isNotEmpty) 'endpoints': endpoints});
}
