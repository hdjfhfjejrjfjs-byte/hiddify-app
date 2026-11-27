// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/route_rules/notifier/rules_notifier.dart (BuiltinRule definitions, _withBuiltins, duplicateRule)
// Replaced by:   rust/crates/hiddify-config (route_rules::builtin_rule_pb_json, builtin_of_rule_name, with_builtins_pb_json, duplicate_rule_pb_json)

import 'package:flutter/foundation.dart';
import 'package:hiddify/core/model/region.dart';
import 'package:hiddify/features/route_rules/notifier/rules_notifier.dart';
import 'package:hiddify/hiddifycore/generated/v2/config/route_rule.pb.dart';

const _geoUrl = 'https://raw.githubusercontent.com/hiddify/hiddify-geo/rule-set';
const _geositeUrl = 'https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set';

Rule legacyBuiltinRule(BuiltinRule kind, TargetPlatform platform) => switch (kind) {
  BuiltinRule.bypassLan => Rule(
    outbound: Outbound.direct,
    ipCidrs: [
      '10.0.0.0/8',
      '172.16.0.0/12',
      '192.168.0.0/16',
      '127.0.0.0/8',
      '169.254.0.0/16',
      '224.0.0.0/4',
      '0.0.0.0/32',
      'fc00::/7',
      'fe80::/10',
      '::1/128',
      'ff00::/8',
      '::/128',
    ],
  ),
  BuiltinRule.systemDirect => switch (platform) {
    TargetPlatform.iOS || TargetPlatform.macOS => Rule(
      outbound: Outbound.direct,
      ruleSets: ['$_geositeUrl/geosite-apple.srs'],
      domainSuffixes: [
        'apple.com',
        'icloud.com',
        'icloud-content.com',
        'apple-dns.net',
        'cdn-apple.com',
        'mzstatic.com',
        'aaplimg.com',
        'itunes.com',
        'apple-cloudkit.com',
        'apple-mapkit.com',
        'testflight.apple.com',
      ],
    ),
    TargetPlatform.windows => Rule(
      outbound: Outbound.direct,
      ruleSets: ['$_geositeUrl/geosite-microsoft.srs'],
      domainSuffixes: [
        'microsoft.com',
        'windows.com',
        'windows.net',
        'windowsupdate.com',
        'msftconnecttest.com',
        'msftncsi.com',
        'live.com',
        'office.com',
        'office.net',
        'microsoftonline.com',
        'onedrive.com',
        'msedge.net',
      ],
    ),
    TargetPlatform.android || TargetPlatform.fuchsia => Rule(
      outbound: Outbound.direct,
      ruleSets: ['$_geositeUrl/geosite-android.srs', '$_geositeUrl/geosite-googlefcm.srs'],
      domainSuffixes: ['android.com', 'mtalk.google.com'],
    ),
    TargetPlatform.linux => Rule(
      outbound: Outbound.direct,
      ruleSets: [
        '$_geositeUrl/geosite-ubuntu.srs',
        '$_geositeUrl/geosite-debian.srs',
        '$_geositeUrl/geosite-fedora.srs',
        '$_geositeUrl/geosite-archlinux.srs',
      ],
      domainSuffixes: [
        'ubuntu.com',
        'debian.org',
        'fedoraproject.org',
        'archlinux.org',
        'flathub.org',
        'snapcraft.io',
      ],
    ),
  },
  BuiltinRule.blockAds => Rule(
    outbound: Outbound.block,
    ruleSets: [
      '$_geoUrl/block/geosite-category-ads-all.srs',
      '$_geoUrl/block/geosite-malware.srs',
      '$_geoUrl/block/geosite-phishing.srs',
      '$_geoUrl/block/geosite-cryptominers.srs',
      '$_geoUrl/block/geoip-malware.srs',
      '$_geoUrl/block/geoip-phishing.srs',
    ],
  ),
  BuiltinRule.bypassRegion => Rule(
    outbound: Outbound.direct,
    ruleSets: ['$_geoUrl/country/geosite-{region}.srs', '$_geoUrl/country/geoip-{region}.srs'],
  ),
}..name = kind.key;

List<Rule> legacyWithBuiltins(List<Rule> saved, Region region, TargetPlatform platform) {
  final missing = {
    for (final builtin in BuiltinRule.values)
      if (builtin != BuiltinRule.bypassRegion || region != Region.other) builtin,
  };
  final rules = <Rule>[];
  for (final rule in saved) {
    final builtin = BuiltinRule.of(rule);
    if (builtin == null) {
      rules.add(rule);
    } else if (missing.remove(builtin)) {
      rules.add(legacyBuiltinRule(builtin, platform)..enabled = rule.enabled);
    }
  }
  final result = [
    for (final builtin in missing) legacyBuiltinRule(builtin, platform)..enabled = builtin.enabledByDefault,
    ...rules,
  ];
  for (var i = 0; i < result.length; i++) {
    result[i].listOrder = i;
  }
  return result;
}
