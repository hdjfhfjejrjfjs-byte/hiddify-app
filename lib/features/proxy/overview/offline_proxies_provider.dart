import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/features/profile/data/profile_data_providers.dart';
import 'package:hiddify/features/profile/notifier/active_profile_notifier.dart';
import 'package:hiddify/features/proxy/keep_top/country_names.dart';
import 'package:hiddify/features/proxy/keep_top/keep_tags_dialog.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top.dart';
import 'package:hiddify/features/proxy/model/proxy_entity.dart';
import 'package:hiddify/features/proxy/overview/proxies_overview_notifier.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'offline_proxies_provider.g.dart';

/// The proxies of the active profile's config file, in config order.
@riverpod
Future<List<ProfileProxy>> activeProfileProxies(Ref ref) async {
  final profileKey = ref.watch(
    activeProfileProvider.select((value) {
      final profile = value.valueOrNull;
      return profile == null ? null : (id: profile.id, lastUpdate: profile.lastUpdate);
    }),
  );
  if (profileKey == null) return const [];
  return readProfileProxies(ref.watch(profilePathResolverProvider).file(profileKey.id));
}

/// Builds a display-ready [OutboundInfo] for an offline [ProfileProxy] read from the profile config.
OutboundInfo offlineProxyToOutboundInfo(ProfileProxy proxy) {
  final code = countryOf(proxy.tag);
  return OutboundInfo(
    tag: proxy.tag,
    tagDisplay: hiddifyRust.require(hiddifyRust.sanitizeTag(proxy.tag), 'sanitizeTag'),
    type: protocolName(proxy.type),
    ipinfo: code != null && code.length == 2 ? IpInfo(countryCode: code) : null,
  );
}

/// Whether [proxy] matches every whitespace-separated token in [query] across its name, tag,
/// protocol type, country code/name, and IP organization.
bool matchesProxyQuery(OutboundInfo proxy, String query) {
  final trimmed = query.trim().toLowerCase();
  if (trimmed.isEmpty) return true;

  final tokens = trimmed.split(RegExp(r'\s+'));
  final ipCountryCode = proxy.ipinfo.countryCode;
  final tagCountryCode = countryOf(proxy.tag);
  final ipCountryName = ipCountryCode.isEmpty ? null : countryNames[ipCountryCode.toUpperCase()];
  final tagCountryName = tagCountryCode == null ? null : countryNames[tagCountryCode.toUpperCase()];

  final fields = <String>[
    proxy.name.toLowerCase(),
    proxy.nameWithDetour.toLowerCase(),
    proxy.tag.toLowerCase(),
    proxy.type.toLowerCase(),
    proxy.typeWithoutStatus.toLowerCase(),
    ipCountryCode.toLowerCase(),
    proxy.ipinfo.org.toLowerCase(),
    (tagCountryCode ?? '').toLowerCase(),
    if (ipCountryName != null) ipCountryName.toLowerCase(),
    if (tagCountryName != null) tagCountryName.toLowerCase(),
  ];

  return tokens.every((token) => fields.any((field) => field.contains(token)));
}

/// Orders offline proxies alphabetically for [ProxiesSort.name], and preserves profile order otherwise.
List<OutboundInfo> sortOfflineProxies(List<OutboundInfo> items, ProxiesSort sortBy) {
  if (sortBy != ProxiesSort.name) return items;
  final keyed = [for (final item in items) (item: item, key: item.name.toLowerCase())]
    ..sort((a, b) {
      final cmp = a.key.compareTo(b.key);
      return cmp != 0 ? cmp : a.item.tag.compareTo(b.item.tag);
    });
  return [for (final entry in keyed) entry.item];
}
