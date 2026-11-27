// ARCHIVED LEGACY DART IMPLEMENTATION (Phase 2)
// Original path: lib/features/settings/data/config_option_repository.dart (keepOwnDnsBehindExtraSecurity)
// Replaced by:   rust/crates/hiddify-config (keep_own_dns_behind_extra_security, merge_options_with_profile_override)

import 'package:hiddify/singbox/model/singbox_config_enum.dart';

Map<String, dynamic> legacyKeepOwnDnsBehindExtraSecurity(Map<String, dynamic> own, Map<String, dynamic> merged) {
  if (merged['chain-status'] != ChainStatus.extraSecurity.key) return merged;
  return {...merged, 'remote-dns-address': own['remote-dns-address']};
}
