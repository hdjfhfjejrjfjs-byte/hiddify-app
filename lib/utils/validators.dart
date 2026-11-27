import 'package:hiddify/core/rust/hiddify_rust.dart';

/// https://stackoverflow.com/questions/3809401/what-is-a-good-regular-expression-to-match-a-url/3809435#3809435
bool isUrl(String input) => hiddifyRust.require(hiddifyRust.isUrl(input), 'isUrl');

bool isPort(String input) => hiddifyRust.require(hiddifyRust.isPort(input), 'isPort');

bool isProcessName(String input) => hiddifyRust.require(hiddifyRust.isProcessName(input), 'isProcessName');

bool isProcessPath(String input) => hiddifyRust.require(hiddifyRust.isProcessPath(input), 'isProcessPath');

bool isPortOrPortRange(String input) => hiddifyRust.require(hiddifyRust.isPortOrPortRange(input), 'isPortOrPortRange');

/// An IPv4 or IPv6 address, with an optional prefix length.
bool isIpCidr(String input) => hiddifyRust.require(hiddifyRust.isIpCidr(input), 'isIpCidr');

bool isDomain(String input) => hiddifyRust.require(hiddifyRust.isDomain(input), 'isDomain');

/// What sing-box's domain_suffix takes: a top-level suffix (`.ir`, `ir`), a whole
/// domain with its subdomains (`noon.com`), or only its subdomains (`.noon.com`).
bool isDomainSuffix(String input) => hiddifyRust.require(hiddifyRust.isDomainSuffix(input), 'isDomainSuffix');
