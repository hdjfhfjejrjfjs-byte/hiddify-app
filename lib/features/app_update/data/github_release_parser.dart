import 'package:hiddify/core/model/environment.dart';
import 'package:hiddify/core/rust/hiddify_rust.dart';
import 'package:hiddify/features/app_update/model/remote_version_entity.dart';

abstract class GithubReleaseParser {
  /// The release, or null when its tag names no version or it is a draft.
  static RemoteVersionEntity? parse(Map<String, dynamic> json) {
    if (json['draft'] == true) return null;
    final fullTag = (json['tag_name'] as String? ?? '').trim();
    hiddifyRust.ensureAvailable('parseReleaseTag');
    final parsedTag = hiddifyRust.parseReleaseTag(fullTag);
    if (parsedTag == null) return null;
    final version = parsedTag.version;
    final suffix = parsedTag.suffix;
    final buildNumber = parsedTag.buildNumber;
    final flavor = suffix == Environment.dev.name ? Environment.dev : Environment.prod;
    final assets = [
      for (final asset in json['assets'] as List? ?? const [])
        if (asset case {'name': final String name, 'browser_download_url': final String url})
          ReleaseAsset(
            name: name,
            url: url,
            size: (asset['size'] as num?)?.toInt() ?? 0,
            digest: asset['digest'] is String ? asset['digest'] as String : null,
          ),
    ];
    return RemoteVersionEntity(
      version: version,
      buildNumber: buildNumber,
      releaseTag: fullTag,
      // the release workflow uploads every release as a GitHub pre-release and it is promoted to
      // a full release once tested: until then only beta users get it, as they get a .dev tag
      preRelease: suffix.isNotEmpty || json['prerelease'] == true,
      url: json['html_url'] as String,
      publishedAt: DateTime.parse(json['published_at'] as String),
      flavor: flavor,
      assets: assets,
      changelog: changelog(json['body'] as String? ?? ''),
    );
  }

  /// The longest change log the dialog shows; past it the release page has the rest.
  static const maxChangelogLength = 4000;

  /// The change log section of a release body in Markdown: from its heading to the next heading
  /// of the same or a higher level, a downloads heading, a `---` rule or the download table. Null when the body has
  /// none or it is empty.
  static String? changelog(String body) {
    hiddifyRust.ensureAvailable('extractChangelog');
    return hiddifyRust.extractChangelog(body);
  }
}
