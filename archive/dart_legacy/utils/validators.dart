// ARCHIVED LEGACY DART IMPLEMENTATION
// Original path: lib/utils/validators.dart
// Replaced by:   rust/crates/hiddify-net (validators)

/// https://gist.github.com/dperini/729294
final _urlRegex = RegExp(
  r'^(?:(?:https?|ftp):\/\/)(?:\S+(?::\S*)?@)?(?:(?:[0-9]{1,3}\.){3}[0-9]{1,3}|(?:(?:[a-z\u00a1-\uffff0-9]+-?)*[a-z\u00a1-\uffff0-9]+)(?:\.(?:[a-z\u00a1-\uffff0-9]+-?)*[a-z\u00a1-\uffff0-9]+)*(?:\.(?:[a-z\u00a1-\uffff]{2,})))(?::\d{2,5})?(?:\/[^\s]*)?#?.*$',
);

/// https://stackoverflow.com/a/12968117
final _portRegex = RegExp(r"^([1-9][0-9]{0,3}|[1-5][0-9]{4}|6[0-4][0-9]{3}|65[0-4][0-9]{2}|655[0-2][0-9]|6553[0-5])$");

final _processNameWindowsRegex = RegExp(r'^[^<>:"\\/|?*]+(?<![\s.])\.(exe|bat|cmd|com)$', caseSensitive: false);
final _processNameLinuxRegex = RegExp(r'^[^\\]+$');
final _processNameMacOSRegex = RegExp(r'^[^\\:]+$');

final _processPathWindowsRegex = RegExp(
  r'^[a-zA-Z]:\\([^<>:"\\/|?*]*(?<!\\)\\)*[^<>:"\\/|?*]+(?<![\s.])\.(exe|bat|cmd|com)$',
  caseSensitive: false,
);
final _processPathLinuxRegex = RegExp(r'^/([^/]*(?<!/)/)*[^/]+$');
final _processPathMacOSRegex = RegExp(r'^/([^/:]*(?<!/)/)*[^/:]+$');

final _portOrPortRangeRegex = RegExp(
  r'^(65000|6[0-4]\d{3}|[1-5]\d{4}|[1-9]\d{0,3})(:(65000|6[0-4]\d{3}|[1-5]\d{4}|[1-9]\d{0,3}))?$',
);

final _cidrPrefixRegex = RegExp(r'^(0|[1-9]\d{0,2})$');
final _domainRegex = RegExp(r'^([a-zA-Z\d\-]+\.)+[a-zA-Z\d\-]{2,}$', caseSensitive: false);
final _domainSuffix = RegExp(r'^\.?([a-zA-Z\d\-_]+\.)*[a-zA-Z\d\-]{2,}$', caseSensitive: false);

bool isUrl(String input) => _urlRegex.hasMatch(input.trim().toLowerCase());

bool isPort(String input) => _portRegex.hasMatch(input);

bool isProcessName(String input) =>
    _processNameWindowsRegex.hasMatch(input) ||
    _processNameLinuxRegex.hasMatch(input) ||
    _processNameMacOSRegex.hasMatch(input);

bool isProcessPath(String input) =>
    _processPathWindowsRegex.hasMatch(input) ||
    _processPathLinuxRegex.hasMatch(input) ||
    _processPathMacOSRegex.hasMatch(input);

bool isPortOrPortRange(String input) => _portOrPortRangeRegex.hasMatch(input);

bool isIpCidr(String input) {
  final parts = input.split('/');
  if (parts.length > 2) return false;
  final isIPv6 = parts.first.contains(':');
  try {
    if (isIPv6) {
      Uri.parseIPv6Address(parts.first);
    } else {
      Uri.parseIPv4Address(parts.first);
    }
  } on FormatException {
    return false;
  }
  if (parts.length == 1) return true;
  return _cidrPrefixRegex.hasMatch(parts.last) && int.parse(parts.last) <= (isIPv6 ? 128 : 32);
}

bool isDomain(String input) => _domainRegex.hasMatch(input);

bool isDomainSuffix(String input) => _domainSuffix.hasMatch(input);
