import 'package:hiddify/core/rust/hiddify_rust.dart';

const String fallbackObscuredAddress = "*.*.*.*";

String obscureIp(String ip) => hiddifyRust.require(hiddifyRust.obscureIp(ip), 'obscureIp');
