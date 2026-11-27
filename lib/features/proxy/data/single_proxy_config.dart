import 'package:hiddify/core/rust/hiddify_rust.dart';

/// A config that holds only one proxy, to share it or edit it as a profile of its own: the proxy,
/// and the proxies it dials through (its `detour`), so it still connects alone.
///
/// The proxy is the one the core runs as [tag], at [host] and [port] when it says (empty and 0
/// when not). It is looked up first in [raw], the profile as saved, by its name: the tag without
/// the core's numbering (`name § 3`), and among proxies of the same name by its port and server.
/// Then in [generated], the profile's generated config, by the exact tag, which matches the
/// running one unless the core renumbered it on a reload, so only when the port agrees. Returns
/// null when neither tells which proxy it is.
///
/// What only the running app knows is left out: a detour to one of the app's own hidden outbounds
/// (such as the TLS fragment one), and `domain_resolver`, which names a DNS server of the app.
String? singleProxyConfig(String tag, {String host = '', int port = 0, String? raw, String? generated}) {
  hiddifyRust.ensureAvailable('singleProxyConfig');
  return hiddifyRust.singleProxyConfig(tag, host: host, port: port, raw: raw, generated: generated);
}

/// [tag] without the core's numbering of duplicate names (`name § 3`).
String baseTag(String tag) => tag.split(' § ').first.trim();

/// Tags of the app's own outbounds, which no shared config has.
bool isHiddenTag(String tag) => tag.contains('§hide§');
