// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/features/proxy/keep_top/keep_tags_dialog.dart (protocolName, countryOf, flagOf)
// Replaced by:   rust/crates/hiddify-config (protocol_name, country_of, flag_of)

String legacyProtocolName(String type) => switch (type) {
  'shadowsocks' => 'Shadowsocks',
  'shadowsocksr' => 'ShadowsocksR',
  'vless' => 'VLESS',
  'vmess' => 'VMess',
  'trojan' => 'Trojan',
  'naive' => 'Naive',
  'wireguard' => 'WireGuard',
  'hysteria' => 'Hysteria',
  'hysteria2' => 'Hysteria2',
  'tuic' => 'TUIC',
  'anytls' => 'AnyTLS',
  'shadowtls' => 'ShadowTLS',
  'ssh' => 'SSH',
  'tor' => 'Tor',
  'socks' => 'SOCKS',
  'http' => 'HTTP',
  'warp' => 'WARP',
  'masque' => 'MASQUE',
  'psiphon' => 'Psiphon',
  'mieru' => 'Mieru',
  'awg' => 'Awg',
  'dnstt' => 'DNSTT',
  'xray' => 'xray',
  '' => '?',
  _ => type[0].toUpperCase() + type.substring(1),
};

String? legacyCountryOf(String tag) {
  bool regional(int rune) => rune >= 0x1F1E6 && rune <= 0x1F1FF;
  final runes = tag.runes.toList();
  for (var i = 0; i + 1 < runes.length; i++) {
    if (regional(runes[i]) && regional(runes[i + 1])) {
      return String.fromCharCodes([runes[i] - 0x1F1E6 + 0x41, runes[i + 1] - 0x1F1E6 + 0x41]);
    }
  }
  return null;
}

String legacyFlagOf(String code) => String.fromCharCodes(code.codeUnits.map((c) => c - 0x41 + 0x1F1E6));
