import 'package:hiddify/core/rust/hiddify_rust.dart';

/// A new secret for the local SOCKS/HTTP port (the user is "hiddify"): [length] characters, a dash
/// after every four (`Kp7x-Q2mZ`), so it reads in parts without a space that is not in it.
String generateProxySecret({int length = 8}) =>
    hiddifyRust.require(hiddifyRust.generateProxySecret(length), 'generateProxySecret');
