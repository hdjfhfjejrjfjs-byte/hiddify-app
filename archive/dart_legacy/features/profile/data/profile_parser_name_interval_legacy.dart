// ARCHIVED LEGACY DART IMPLEMENTATION (Final Overlap Audit)
// Original path: lib/features/profile/data/profile_parser.dart (parse() 7-level name hierarchy & update interval)
//                & lib/features/proxy/keep_top/keep_tags_dialog.dart (inline tagDisplay §-strip)
// Replaced by:   rust/crates/hiddify-profile (resolve_profile_name_from_path, parse_update_interval_hours)
//                & rust/crates/hiddify-net (sanitize_tag)

import 'dart:io';

import 'package:dartx/dartx.dart';
import 'package:hiddify/features/profile/data/profile_parser.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';

String legacyResolveProfileName({
  required Map<String, dynamic> headers,
  required ProfileEntity profile,
  required String tempFilePath,
}) {
  var name = '';
  if (profile.userOverride?.name case final String oName when oName.isNotBlank) {
    name = oName;
  }
  if (headers['profile-title'] case final String titleHeader when name.isBlank) {
    if (titleHeader.startsWith("base64:")) {
      name = ProfileParser.decodeBase64Title(titleHeader.replaceFirst("base64:", ""));
    } else {
      name = titleHeader.trim();
    }
  }
  if (headers['content-disposition'] case final String contentDispositionHeader when name.isBlank) {
    name = ProfileParser.filenameFromContentDisposition(contentDispositionHeader);
  }
  if (profile case RemoteProfileEntity(:final url)) {
    if (Uri.parse(url).fragment case final fragment when name.isBlank) {
      name = fragment;
    }
    if (url.split("/").lastOrNull case final part? when name.isBlank) {
      final pattern = RegExp(r"\.(json|yaml|yml|txt)[\s\S]*");
      name = part.replaceFirst(pattern, "");
    }
  }
  if (name.isBlank) {
    switch (profile) {
      case RemoteProfileEntity():
        name = "Remote Profile";
      case LocalProfileEntity():
        name = profile.name.isNotBlank ? profile.name : ProfileParser.protocol(File(tempFilePath).readAsStringSync());
    }
  }
  return name;
}

ProfileOptions? legacyParseUpdateInterval({
  required Map<String, dynamic> headers,
  required ProfileEntity profile,
}) {
  final isAutoUpdateDisable = profile.userOverride?.isAutoUpdateDisable ?? false;
  ProfileOptions? options;
  if (profile.userOverride?.updateInterval case final int updateInterval
      when updateInterval > 0 && !isAutoUpdateDisable) {
    options = ProfileOptions(updateInterval: Duration(hours: updateInterval));
  }
  if (headers['profile-update-interval'] case final String updateIntervalStr
      when options == null && !isAutoUpdateDisable) {
    if (int.tryParse(updateIntervalStr.trim()) case final hours? when hours > 0) {
      options = ProfileOptions(updateInterval: Duration(hours: hours.clamp(1, maxUpdateIntervalHours)));
    }
  }
  return options;
}

String legacyKeepTagsDisplay(String tag) => tag.replaceFirst(RegExp('§.*'), '').trimRight();
