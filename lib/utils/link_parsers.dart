import 'package:hiddify/core/rust/hiddify_rust.dart';

typedef ProfileLink = ({String url, String name});

// TODO: test and improve
abstract class LinkParser {
  static String generateSubShareLink(String url, [String? name]) =>
      hiddifyRust.require(hiddifyRust.generateSubShareLink(url, name), 'generateSubShareLink');

  // protocols schemas
  static const protocols = ['hiddify', 'v2ray', 'v2rayn', 'v2rayng', 'clash', 'clashmeta', 'sing-box'];

  static ProfileLink? parse(String link) {
    hiddifyRust.ensureAvailable('parseLink');
    return hiddifyRust.parseLink(link);
  }

  static ProfileLink? simple(String link) {
    hiddifyRust.ensureAvailable('parseSimpleLink');
    return hiddifyRust.parseSimpleLink(link);
  }

  static ProfileLink? deep(String link) {
    hiddifyRust.ensureAvailable('parseDeepLink');
    return hiddifyRust.parseDeepLink(link);
  }
}

String safeDecodeBase64(String str) => hiddifyRust.require(hiddifyRust.safeDecodeBase64(str), 'safeDecodeBase64');
