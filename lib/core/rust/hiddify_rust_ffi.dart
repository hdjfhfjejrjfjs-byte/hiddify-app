// ignore_for_file: avoid_private_typedef_functions

import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

typedef _AbiVersionNative = ffi.Uint32 Function();
typedef _AbiVersionDart = int Function();

typedef _FreeStringNative = ffi.Void Function(ffi.Pointer<Utf8>);
typedef _FreeStringDart = void Function(ffi.Pointer<Utf8>);

typedef _NoArgStrNative = ffi.Pointer<Utf8> Function();
typedef _NoArgStrDart = ffi.Pointer<Utf8> Function();

typedef _StrToStrNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);
typedef _StrToStrDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);

typedef _Str2ToStrNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Str2ToStrDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

typedef _StrToBoolNative = ffi.Int32 Function(ffi.Pointer<Utf8>);
typedef _StrToBoolDart = int Function(ffi.Pointer<Utf8>);

typedef _StrToI64Native = ffi.Int64 Function(ffi.Pointer<Utf8>);
typedef _StrToI64Dart = int Function(ffi.Pointer<Utf8>);

typedef _SizeToStrNative = ffi.Pointer<Utf8> Function(ffi.Size);
typedef _SizeToStrDart = ffi.Pointer<Utf8> Function(int);

typedef _IsIdentityUsableNative = ffi.Int32 Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Uint64);
typedef _IsIdentityUsableDart = int Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int);

typedef _VerifyPinnedCertNative =
    ffi.Int32 Function(
      ffi.Pointer<ffi.Uint8>,
      ffi.Size,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );
typedef _VerifyPinnedCertDart =
    int Function(
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );

typedef _VerifyFileSha256Native = ffi.Int32 Function(ffi.Pointer<Utf8>, ffi.Uint64, ffi.Pointer<Utf8>);
typedef _VerifyFileSha256Dart = int Function(ffi.Pointer<Utf8>, int, ffi.Pointer<Utf8>);

typedef _SingleProxyConfigNative =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Int64,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );
typedef _SingleProxyConfigDart =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      int,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );

typedef _BytesToStrNative = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Uint8>, ffi.Size);
typedef _BytesToStrDart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Uint8>, int);

typedef _TelegramPayloadNative =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );
typedef _TelegramPayloadDart =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );

typedef _CrashExcerptNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Bool);
typedef _CrashExcerptDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, bool);

typedef _Str2ToBoolNative = ffi.Int32 Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Str2ToBoolDart = int Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

typedef _BytesToBoolNative = ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.Size);
typedef _BytesToBoolDart = int Function(ffi.Pointer<ffi.Uint8>, int);

typedef _FormParseNative = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Uint8>, ffi.Size, ffi.Size);
typedef _FormParseDart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Uint8>, int, int);

typedef _FormatRangeNative = ffi.Pointer<Utf8> Function(ffi.Bool, ffi.Int64, ffi.Bool, ffi.Int64);
typedef _FormatRangeDart = ffi.Pointer<Utf8> Function(bool, int, bool, int);

typedef _Str3ToStrNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Str3ToStrDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

typedef _DupRuleNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Int64);
typedef _DupRuleDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, int);

typedef _I32ToI32Native = ffi.Int32 Function(ffi.Int32);
typedef _I32ToI32Dart = int Function(int);

typedef _I32PairToI32Native = ffi.Int32 Function(ffi.Int32, ffi.Int32);
typedef _I32PairToI32Dart = int Function(int, int);

typedef _ClassifyDioNative =
    ffi.Int32 Function(ffi.Int32, ffi.Pointer<Utf8>, ffi.Bool, ffi.Bool, ffi.Pointer<Utf8>);
typedef _ClassifyDioDart = int Function(int, ffi.Pointer<Utf8>, bool, bool, ffi.Pointer<Utf8>);

typedef _MostTellingNative = ffi.Int32 Function(ffi.Pointer<ffi.Int32>, ffi.Size);
typedef _MostTellingDart = int Function(ffi.Pointer<ffi.Int32>, int);

typedef _I32PairToStrNative = ffi.Pointer<Utf8> Function(ffi.Int32, ffi.Int32);
typedef _I32PairToStrDart = ffi.Pointer<Utf8> Function(int, int);

typedef _NoArgBoolNative = ffi.Int32 Function();
typedef _NoArgBoolDart = int Function();

typedef _NoArgVoidNative = ffi.Void Function();
typedef _NoArgVoidDart = void Function();

typedef _Str2VoidNative = ffi.Void Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Str2VoidDart = void Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

typedef _OwnFramesNative =
    ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Size);
typedef _OwnFramesDart =
    ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int);

typedef _AssembleDiagNative =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );
typedef _AssembleDiagDart =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );

typedef _CrashReasonOfNative =
    ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Int64);
typedef _CrashReasonOfDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int);

typedef _I64ToStrNative = ffi.Pointer<Utf8> Function(ffi.Int64);
typedef _I64ToStrDart = ffi.Pointer<Utf8> Function(int);

typedef _CoreGoneNative = ffi.Int32 Function(ffi.Int32, ffi.Pointer<Utf8>);
typedef _CoreGoneDart = int Function(int, ffi.Pointer<Utf8>);

typedef _ResolveNameNative =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );
typedef _ResolveNameDart =
    ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
      ffi.Pointer<Utf8>,
    );

typedef _UpdateIntervalNative = ffi.Int64 Function(ffi.Pointer<Utf8>, ffi.Bool, ffi.Int64, ffi.Bool);
typedef _UpdateIntervalDart = int Function(ffi.Pointer<Utf8>, bool, int, bool);

typedef _NoArgU64Native = ffi.Uint64 Function();
typedef _NoArgU64Dart = int Function();

typedef _U64ToBoolNative = ffi.Int32 Function(ffi.Uint64);
typedef _U64ToBoolDart = int Function(int);

typedef _U64VoidNative = ffi.Void Function(ffi.Uint64);
typedef _U64VoidDart = void Function(int);

typedef _U64ToStrNative = ffi.Pointer<Utf8> Function(ffi.Uint64);
typedef _U64ToStrDart = ffi.Pointer<Utf8> Function(int);

/// Cross-platform `dart:ffi` loader and bindings for `libhiddify_rust` across
/// iOS (`XCFramework` static symbols), Android (`jniLibs`), macOS (`dylib`/`XCFramework`), and Linux (`bundle/lib`).
class HiddifyRustNative {
  HiddifyRustNative._() : _bindings = _tryLoad();

  static final HiddifyRustNative instance = HiddifyRustNative._();

  final _Bindings? _bindings;

  bool get isAvailable => _bindings != null;

  static _Bindings? _tryLoad() {
    for (final candidate in _candidateLibraries()) {
      try {
        final lib = candidate();
        final b = _Bindings(lib);
        if (b.abiVersion() == 5) {
          return b;
        }
      } catch (_) {
        continue;
      }
    }
    return null;
  }

  static Iterable<ffi.DynamicLibrary Function()> _candidateLibraries() sync* {
    final envPath = Platform.environment['HIDDIFY_RUST_LIB_PATH'];
    if (envPath != null && envPath.isNotEmpty) {
      yield () => ffi.DynamicLibrary.open(envPath);
    }

    if (Platform.isIOS) {
      // Dynamic framework embedded via HiddifyRust.xcframework (never stripped in Release),
      // falling back to process/executable symbols for static/test builds.
      yield () => ffi.DynamicLibrary.open('HiddifyRust.framework/HiddifyRust');
      yield ffi.DynamicLibrary.process;
      yield ffi.DynamicLibrary.executable;
      return;
    }

    if (Platform.isAndroid) {
      yield () => ffi.DynamicLibrary.open('libhiddify_rust.so');
      return;
    }

    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final cwd = Directory.current.path;

    if (Platform.isMacOS) {
      yield () => ffi.DynamicLibrary.open('$exeDir/../Frameworks/libhiddify_rust.dylib');
      yield () => ffi.DynamicLibrary.open('libhiddify_rust.dylib');
      yield ffi.DynamicLibrary.process;
      yield () => ffi.DynamicLibrary.open('$cwd/macos/Frameworks/libhiddify_rust.dylib');
      yield () => ffi.DynamicLibrary.open('$cwd/rust/target/release/libhiddify_rust.dylib');
      return;
    }

    if (Platform.isLinux) {
      yield () => ffi.DynamicLibrary.open('$exeDir/lib/libhiddify_rust.so');
      yield () => ffi.DynamicLibrary.open('libhiddify_rust.so');
      yield () => ffi.DynamicLibrary.open('$cwd/rust/dist/linux/libhiddify_rust.so');
      yield () => ffi.DynamicLibrary.open('$cwd/rust/target/release/libhiddify_rust.so');
      return;
    }

    if (Platform.isWindows) {
      yield () => ffi.DynamicLibrary.open('$exeDir/hiddify_rust.dll');
      yield () => ffi.DynamicLibrary.open('hiddify_rust.dll');
      yield () => ffi.DynamicLibrary.open('$cwd/rust/target/release/hiddify_rust.dll');
    }
  }

  String? _readAndFree(ffi.Pointer<Utf8> ptr) {
    if (ptr == ffi.nullptr) return null;
    try {
      return ptr.toDartString();
    } finally {
      _bindings!.freeString(ptr);
    }
  }

  bool? _toBool(int code) => code == 1 ? true : (code == 0 ? false : null);

  bool? _callStrBool(_StrToBoolDart fn, String input) {
    return using((arena) => _toBool(fn(input.toNativeUtf8(allocator: arena))));
  }

  String? _callStrToStr(_StrToStrDart fn, String input) {
    return using((arena) => _readAndFree(fn(input.toNativeUtf8(allocator: arena))));
  }

  // --- 1. Crypto ---

  ({String certificatePem, String privateKeyPem})? generateIdentity() {
    final b = _bindings;
    if (b == null) return null;
    final raw = _readAndFree(b.generateIdentityJson());
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      certificatePem: map['certificatePem'] as String,
      privateKeyPem: map['privateKeyPem'] as String,
    );
  }

  ({String certificatePem, String privateKeyPem})? loadOrCreateIdentity(String directoryPath) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.loadOrCreateIdentityJson, directoryPath);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      certificatePem: map['certificatePem'] as String,
      privateKeyPem: map['privateKeyPem'] as String,
    );
  }

  bool? isIdentityUsable(String certificatePem, String privateKeyPem, int renewBeforeSecs) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final code = b.isIdentityUsable(
        certificatePem.toNativeUtf8(allocator: arena),
        privateKeyPem.toNativeUtf8(allocator: arena),
        renewBeforeSecs,
      );
      return _toBool(code);
    });
  }

  bool? verifyPinnedCert(Uint8List presentedDer, String presentedPem, String expectedServerPem, String host) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final derPtr = arena<ffi.Uint8>(presentedDer.isEmpty ? 1 : presentedDer.length);
      if (presentedDer.isNotEmpty) {
        derPtr.asTypedList(presentedDer.length).setAll(0, presentedDer);
      }
      final code = b.verifyPinnedCert(
        presentedDer.isEmpty ? ffi.nullptr : derPtr,
        presentedDer.length,
        presentedPem.toNativeUtf8(allocator: arena),
        expectedServerPem.toNativeUtf8(allocator: arena),
        host.toNativeUtf8(allocator: arena),
      );
      return _toBool(code);
    });
  }

  String? generateProxySecret(int length) {
    final b = _bindings;
    if (b == null) return null;
    return _readAndFree(b.generateProxySecret(length));
  }

  bool? isValidSha256Digest(String digest) {
    final b = _bindings;
    if (b == null) return null;
    return _callStrBool(b.isValidSha256Digest, digest);
  }

  bool? verifyFileSha256(String filePath, int expectedSize, String expectedDigest) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final code = b.verifyFileSha256(
        filePath.toNativeUtf8(allocator: arena),
        expectedSize,
        expectedDigest.toNativeUtf8(allocator: arena),
      );
      return _toBool(code);
    });
  }

  // --- 2. Net ---

  bool? isUrl(String input) => _bindings == null ? null : _callStrBool(_bindings.isUrl, input);
  bool? isPort(String input) => _bindings == null ? null : _callStrBool(_bindings.isPort, input);
  bool? isPortOrPortRange(String input) =>
      _bindings == null ? null : _callStrBool(_bindings.isPortOrPortRange, input);
  bool? isProcessName(String input) => _bindings == null ? null : _callStrBool(_bindings.isProcessName, input);
  bool? isProcessPath(String input) => _bindings == null ? null : _callStrBool(_bindings.isProcessPath, input);
  bool? isIpCidr(String input) => _bindings == null ? null : _callStrBool(_bindings.isIpCidr, input);
  bool? isDomain(String input) => _bindings == null ? null : _callStrBool(_bindings.isDomain, input);
  bool? isDomainSuffix(String input) => _bindings == null ? null : _callStrBool(_bindings.isDomainSuffix, input);
  String? obscureIp(String ip) => _bindings == null ? null : _callStrToStr(_bindings.obscureIp, ip);
  String? sanitizeTag(String tag) => _bindings == null ? null : _callStrToStr(_bindings.sanitizeTag, tag);
  String? stripStatusMark(String proxyType) =>
      _bindings == null ? null : _callStrToStr(_bindings.stripStatusMark, proxyType);
  bool? isInvalidProxyType(String typeWithoutStatus) =>
      _bindings == null ? null : _callStrBool(_bindings.isInvalidProxyType, typeWithoutStatus);

  ({String before, String endpoint, String after})? splitScannerEndpoint(String proxyType) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.splitScannerEndpointJson, proxyType);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      before: map['before'] as String,
      endpoint: map['endpoint'] as String,
      after: map['after'] as String,
    );
  }

  // --- 3. Config ---

  List<({String tag, String type})>? profileProxies(String configJson) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.profileProxiesJson, configJson);
    if (raw == null) throw const FormatException('Invalid profile JSON');
    final list = jsonDecode(raw) as List<dynamic>;
    return [
      for (final item in list)
        if (item is Map<String, dynamic>) (tag: item['tag'] as String, type: item['type'] as String),
    ];
  }

  int? countProxies(String configJson) {
    final b = _bindings;
    if (b == null) return null;
    final res = using((arena) => b.countProxies(configJson.toNativeUtf8(allocator: arena)));
    if (res < 0) throw const FormatException('Invalid profile JSON');
    return res;
  }

  String? countryOf(String tag) {
    final b = _bindings;
    if (b == null) return null;
    return _callStrToStr(b.countryOf, tag);
  }

  String? flagOf(String code) => _bindings == null ? null : _callStrToStr(_bindings.flagOf, code);
  String? protocolName(String proxyType) =>
      _bindings == null ? null : _callStrToStr(_bindings.protocolName, proxyType);

  ({String text, bool isJson})? editableConfig(String content) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.editableConfigJson, content);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (text: map['text'] as String, isJson: map['is_json'] as bool);
  }

  String? singleProxyConfig(String tag, {String host = '', int port = 0, String? raw, String? generated}) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final ptr = b.singleProxyConfig(
        tag.toNativeUtf8(allocator: arena),
        host.toNativeUtf8(allocator: arena),
        port,
        raw == null ? ffi.nullptr : raw.toNativeUtf8(allocator: arena),
        generated == null ? ffi.nullptr : generated.toNativeUtf8(allocator: arena),
      );
      return _readAndFree(ptr);
    });
  }

  String? profileOverride(Map<String, dynamic>? populatedHeaders, Map<String, dynamic>? userOverride) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final hPtr = populatedHeaders == null
          ? ffi.nullptr
          : jsonEncode(populatedHeaders).toNativeUtf8(allocator: arena);
      final uPtr = userOverride == null ? ffi.nullptr : jsonEncode(userOverride).toNativeUtf8(allocator: arena);
      return _readAndFree(b.profileOverride(hPtr, uPtr));
    });
  }

  Map<String, dynamic>? applyProfileOverride(Map<String, dynamic> main, String? profileOverride) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final mPtr = jsonEncode(main).toNativeUtf8(allocator: arena);
      final oPtr = profileOverride == null ? ffi.nullptr : profileOverride.toNativeUtf8(allocator: arena);
      final raw = _readAndFree(b.applyProfileOverride(mPtr, oPtr));
      if (raw == null) return null;
      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }

  // --- 4. Profile ---

  String? safeDecodeBase64(String input) =>
      _bindings == null ? null : _callStrToStr(_bindings.safeDecodeBase64, input);
  String? decodeBase64Title(String value) =>
      _bindings == null ? null : _callStrToStr(_bindings.decodeBase64Title, value);
  String? filenameFromContentDisposition(String header) =>
      _bindings == null ? null : _callStrToStr(_bindings.filenameFromContentDisposition, header);

  Map<String, dynamic>? populateHeaders(String content, Map<String, dynamic>? remoteHeaders) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final cPtr = content.toNativeUtf8(allocator: arena);
      final rPtr = remoteHeaders == null ? ffi.nullptr : jsonEncode(remoteHeaders).toNativeUtf8(allocator: arena);
      final raw = _readAndFree(b.populateHeadersJson(cPtr, rPtr));
      if (raw == null) return null;
      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }

  ({int upload, int download, int total, int expireSecs})? parseSubscriptionInfo(String subInfoStr) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.parseSubscriptionInfoJson, subInfoStr);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      upload: (map['upload'] as num).toInt(),
      download: (map['download'] as num).toInt(),
      total: (map['total'] as num).toInt(),
      expireSecs: (map['expire_secs'] as num).toInt(),
    );
  }

  String? detectProtocol(String content) =>
      _bindings == null ? null : _callStrToStr(_bindings.detectProtocol, content);
  bool? isSubscriptionLink(String line) =>
      _bindings == null ? null : _callStrBool(_bindings.isSubscriptionLink, line);

  ({String url, String name})? _decodeLink(String? raw) {
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (url: map['url'] as String, name: map['name'] as String);
  }

  ({String url, String name})? parseLink(String link) =>
      _bindings == null ? null : _decodeLink(_callStrToStr(_bindings.parseLinkJson, link));
  ({String url, String name})? parseSimpleLink(String link) =>
      _bindings == null ? null : _decodeLink(_callStrToStr(_bindings.parseSimpleLinkJson, link));
  ({String url, String name})? parseDeepLink(String link) =>
      _bindings == null ? null : _decodeLink(_callStrToStr(_bindings.parseDeepLinkJson, link));

  String? generateSubShareLink(String url, String? name) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final uPtr = url.toNativeUtf8(allocator: arena);
      final nPtr = name == null ? ffi.nullptr : name.toNativeUtf8(allocator: arena);
      return _readAndFree(b.generateSubShareLink(uPtr, nPtr));
    });
  }

  String? _callBytesToStr(_BytesToStrDart fn, Uint8List bytes) {
    return using((arena) {
      final ptr = arena<ffi.Uint8>(bytes.isEmpty ? 1 : bytes.length);
      if (bytes.isNotEmpty) {
        ptr.asTypedList(bytes.length).setAll(0, bytes);
      }
      return _readAndFree(fn(bytes.isEmpty ? ffi.nullptr : ptr, bytes.length));
    });
  }

  String? decodeTextBytes(Uint8List bytes) =>
      _bindings == null ? null : _callBytesToStr(_bindings.decodeTextBytes, bytes);
  String? readZipText(Uint8List bytes) => _bindings == null ? null : _callBytesToStr(_bindings.readZipText, bytes);

  // --- 5. Diagnostics ---

  String? telegramPayload({
    required String os,
    required String ov,
    required String v,
    required String t,
    String? pm,
    String? l,
    String? a,
  }) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      ffi.Pointer<Utf8> optStr(String? s) => s == null ? ffi.nullptr : s.toNativeUtf8(allocator: arena);
      return _readAndFree(
        b.telegramPayload(
          os.toNativeUtf8(allocator: arena),
          ov.toNativeUtf8(allocator: arena),
          v.toNativeUtf8(allocator: arena),
          t.toNativeUtf8(allocator: arena),
          optStr(pm),
          optStr(l),
          optStr(a),
        ),
      );
    });
  }

  String? windowsVersion(String osVersion) =>
      _bindings == null ? null : _callStrToStr(_bindings.windowsVersion, osVersion);

  (String, String)? linuxDistro(String osRelease) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.linuxDistroJson, osRelease);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (map['code'] as String, map['version'] as String);
  }

  ({String headline, String text})? crashExcerpt(String content, {required bool findGoMarker}) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final raw = _readAndFree(b.crashExcerptJson(content.toNativeUtf8(allocator: arena), findGoMarker));
      if (raw == null) return null;
      final map = jsonDecode(raw) as Map<String, dynamic>;
      return (headline: map['headline'] as String, text: map['text'] as String);
    });
  }

  String? extractChangelog(String body) => _bindings == null ? null : _callStrToStr(_bindings.extractChangelog, body);

  ({String version, String suffix, String buildNumber})? parseReleaseTag(String tagName) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.parseReleaseTagJson, tagName);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      version: map['version'] as String,
      suffix: map['suffix'] as String,
      buildNumber: map['build_number'] as String,
    );
  }

  // --- 6. Phase 2 Extensions ---

  String? generateNumericPin(int length) =>
      _bindings == null ? null : _readAndFree(_bindings.generateNumericPin(length));

  String? generateUrlToken(int byteLen) =>
      _bindings == null ? null : _readAndFree(_bindings.generateUrlToken(byteLen));

  String? generateAlphanumericSecret(int length) =>
      _bindings == null ? null : _readAndFree(_bindings.generateAlphanumericSecret(length));

  bool? verifySecretPath(String path, String token) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      return _toBool(
        b.verifySecretPath(path.toNativeUtf8(allocator: arena), token.toNativeUtf8(allocator: arena)),
      );
    });
  }

  bool? isLocalNetworkBytes(Uint8List rawIp) {
    final b = _bindings;
    if (b == null) return null;
    if (rawIp.isEmpty) return false;
    return using((arena) {
      final ptr = arena<ffi.Uint8>(rawIp.length);
      ptr.asTypedList(rawIp.length).setAll(0, rawIp);
      return _toBool(b.isLocalNetworkBytes(ptr, rawIp.length));
    });
  }

  bool? isAllowedHostHeader(String? hostHeader, String expectedAuthority) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final hPtr = hostHeader == null ? ffi.nullptr : hostHeader.toNativeUtf8(allocator: arena);
      return _toBool(b.isAllowedHostHeader(hPtr, expectedAuthority.toNativeUtf8(allocator: arena)));
    });
  }

  ({String status, String? text})? parsePhoneImportForm(Uint8List body, int maxTextBytes) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using((arena) {
      final ptr = arena<ffi.Uint8>(body.isEmpty ? 1 : body.length);
      if (body.isNotEmpty) {
        ptr.asTypedList(body.length).setAll(0, body);
      }
      return _readAndFree(b.parsePhoneImportFormJson(body.isEmpty ? ffi.nullptr : ptr, body.length, maxTextBytes));
    });
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (status: map['status'] as String, text: map['text'] as String?);
  }

  ({int? min, int? max})? parseOptionalRange(String input, {required bool allowEmpty}) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using(
      (arena) => _readAndFree(b.parseOptionalRangeJson(input.toNativeUtf8(allocator: arena), allowEmpty)),
    );
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (min: (map['min'] as num?)?.toInt(), max: (map['max'] as num?)?.toInt());
  }

  String? formatOptionalRange(int? min, int? max) {
    final b = _bindings;
    if (b == null) return null;
    return _readAndFree(b.formatOptionalRange(min != null, min ?? 0, max != null, max ?? 0));
  }

  String? extraSecurityMode(String? value) {
    final b = _bindings;
    if (b == null || value == null) return null;
    return _callStrToStr(b.extraSecurityMode, value);
  }

  int? keepTopHeader(String? value) {
    final b = _bindings;
    if (b == null || value == null) return null;
    final res = using((arena) => b.keepTopHeader(value.toNativeUtf8(allocator: arena)));
    return res == -9999 ? null : res;
  }

  Map<String, dynamic>? fragmentTricks(String? value) {
    final b = _bindings;
    if (b == null || value == null) return null;
    final raw = _callStrToStr(b.fragmentTricksJson, value);
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  Map<String, dynamic>? keepOwnDnsBehindExtraSecurity(Map<String, dynamic> own, Map<String, dynamic> merged) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final raw = _readAndFree(
        b.keepOwnDnsJson(
          jsonEncode(own).toNativeUtf8(allocator: arena),
          jsonEncode(merged).toNativeUtf8(allocator: arena),
        ),
      );
      if (raw == null) return null;
      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }

  Map<String, dynamic>? mergeOptionsWithProfileOverride(Map<String, dynamic> own, String? profileOverride) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final oPtr = profileOverride == null ? ffi.nullptr : profileOverride.toNativeUtf8(allocator: arena);
      final raw = _readAndFree(
        b.mergeOptionsWithOverrideJson(jsonEncode(own).toNativeUtf8(allocator: arena), oPtr),
      );
      if (raw == null) return null;
      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }

  String? builtinRulePbJson(String shortName, String platform) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _readAndFree(
        b.builtinRulePbJson(
          shortName.toNativeUtf8(allocator: arena),
          platform.toNativeUtf8(allocator: arena),
        ),
      ),
    );
  }

  String? builtinOfRuleName(String ruleName) =>
      _bindings == null ? null : _callStrToStr(_bindings.builtinOfRuleName, ruleName);

  String? withBuiltinsPbJson(String savedPbJson, String region, String platform) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _readAndFree(
        b.withBuiltinsPbJson(
          savedPbJson.toNativeUtf8(allocator: arena),
          region.toNativeUtf8(allocator: arena),
          platform.toNativeUtf8(allocator: arena),
        ),
      ),
    );
  }

  ({String pbJson, int order})? duplicateRulePbJson(String currentPbJson, int listOrder) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using(
      (arena) => _readAndFree(b.duplicateRulePbJson(currentPbJson.toNativeUtf8(allocator: arena), listOrder)),
    );
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (pbJson: map['pb_json'] as String, order: (map['order'] as num).toInt());
  }

  int? downloadProblemOfStatus(int code) =>
      _bindings == null ? null : _bindings.downloadProblemOfStatus(code);

  int? downloadProblemOfCore(int fetchError, int statusCode) =>
      _bindings == null ? null : _bindings.downloadProblemOfCore(fetchError, statusCode);

  int? downloadProblemClassifyDio({
    int? statusCode,
    required String dioType,
    required bool isCertExc,
    required bool isTlsExc,
    required String text,
  }) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => b.downloadProblemClassifyDio(
        statusCode ?? -1,
        dioType.toNativeUtf8(allocator: arena),
        isCertExc,
        isTlsExc,
        text.toNativeUtf8(allocator: arena),
      ),
    );
  }

  int? downloadProblemMostTelling(List<int> problemIndices) {
    final b = _bindings;
    if (b == null) return null;
    if (problemIndices.isEmpty) return 13;
    return using((arena) {
      final ptr = arena<ffi.Int32>(problemIndices.length);
      for (var i = 0; i < problemIndices.length; i++) {
        ptr[i] = problemIndices[i];
      }
      return b.downloadProblemMostTelling(ptr, problemIndices.length);
    });
  }

  ({bool mayPass, String code, int? failedStop, int side})? downloadProblemMeta(int problemIndex, int? statusCode) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _readAndFree(b.downloadProblemMetaJson(problemIndex, statusCode ?? -1));
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      mayPass: map['may_pass'] as bool,
      code: map['code'] as String,
      failedStop: (map['failed_stop'] as num?)?.toInt(),
      side: (map['side'] as num).toInt(),
    );
  }

  Map<String, dynamic>? flattenHttpHeaders(Map<String, List<String>> headers) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.flattenHeadersJson, jsonEncode(headers));
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  ({String decodedText, List<({int index, String url})> targets})? planLinkExpansion(
    String rawText, {
    required bool decodeBase64First,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using(
      (arena) => _readAndFree(b.planLinkExpansionJson(rawText.toNativeUtf8(allocator: arena), decodeBase64First)),
    );
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    final targets = [
      for (final item in map['targets'] as List<dynamic>)
        if (item is Map<String, dynamic>) (index: (item['index'] as num).toInt(), url: item['url'] as String),
    ];
    return (decodedText: map['decoded_text'] as String, targets: targets);
  }

  String? mergeExpandedLinks(String decodedText, List<(int, String)> replacements) {
    final b = _bindings;
    if (b == null) return null;
    final repList = [
      for (final (idx, body) in replacements) [idx, body],
    ];
    return using(
      (arena) => _readAndFree(
        b.mergeExpandedLinks(
          decodedText.toNativeUtf8(allocator: arena),
          jsonEncode(repList).toNativeUtf8(allocator: arena),
        ),
      ),
    );
  }

  String? resolveProfileName({
    String? overrideName,
    String? profileTitleHeader,
    String? contentDispositionHeader,
    String? remoteUrl,
    String? existingLocalName,
    required String tempFilePath,
  }) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      ffi.Pointer<Utf8> optStr(String? s) => s == null ? ffi.nullptr : s.toNativeUtf8(allocator: arena);
      return _readAndFree(
        b.resolveProfileName(
          optStr(overrideName),
          optStr(profileTitleHeader),
          optStr(contentDispositionHeader),
          optStr(remoteUrl),
          optStr(existingLocalName),
          tempFilePath.toNativeUtf8(allocator: arena),
        ),
      );
    });
  }

  int? parseUpdateIntervalHours({
    String? headerValue,
    int? overrideHours,
    required bool isAutoUpdateDisable,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final res = using((arena) {
      final hPtr = headerValue == null ? ffi.nullptr : headerValue.toNativeUtf8(allocator: arena);
      return b.parseUpdateIntervalHours(hPtr, overrideHours != null, overrideHours ?? 0, isAutoUpdateDisable);
    });
    return res > 0 ? res : null;
  }

  bool? openLogFile(String path) => _bindings == null ? null : _callStrBool(_bindings.openLogFile, path);

  bool? isLogFileOpen() => _bindings == null ? null : _toBool(_bindings.isLogFileOpen());

  void writeLogLine(String line) {
    final b = _bindings;
    if (b == null) return;
    using((arena) => b.writeLogLine(line.toNativeUtf8(allocator: arena)));
  }

  void closeLogFile() => _bindings?.closeLogFile();

  bool? writeTextToFile(String path, String content) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _toBool(
        b.writeTextToFile(path.toNativeUtf8(allocator: arena), content.toNativeUtf8(allocator: arena)),
      ),
    );
  }

  String? originOfStack(String stack, String ownPrefix, String loggerInternals) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _readAndFree(
        b.originOfStack(
          stack.toNativeUtf8(allocator: arena),
          ownPrefix.toNativeUtf8(allocator: arena),
          loggerInternals.toNativeUtf8(allocator: arena),
        ),
      ),
    );
  }

  String? ownStackFrames(String stack, String ownPrefix, String loggerInternals, int maxFrames) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _readAndFree(
        b.ownStackFrames(
          stack.toNativeUtf8(allocator: arena),
          ownPrefix.toNativeUtf8(allocator: arena),
          loggerInternals.toNativeUtf8(allocator: arena),
          maxFrames,
        ),
      ),
    );
  }

  bool? openAppCrashStore(String workingDir) =>
      _bindings == null ? null : _callStrBool(_bindings.openAppCrashStore, workingDir);

  void recordAppCrash(String kind, String text) {
    final b = _bindings;
    if (b == null) return;
    using((arena) => b.recordAppCrash(kind.toNativeUtf8(allocator: arena), text.toNativeUtf8(allocator: arena)));
  }

  List<String>? listAppCrashFiles(String workingDir) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.listAppCrashFilesJson, workingDir);
    if (raw == null) return null;
    return (jsonDecode(raw) as List<dynamic>).cast<String>();
  }

  List<({String name, String source, String crashedAt})>? listFatalAppCrashReports(String workingDir) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.listFatalAppCrashReportsJson, workingDir);
    if (raw == null) return null;
    return [
      for (final item in jsonDecode(raw) as List<dynamic>)
        if (item is Map<String, dynamic>)
          (
            name: item['name'] as String,
            source: item['source'] as String,
            crashedAt: item['crashed_at'] as String,
          ),
    ];
  }

  Map<String, Object?>? scrubPreferences(Map<String, Object?> prefs) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.scrubPreferencesJson, jsonEncode(prefs));
    if (raw == null) return null;
    return (jsonDecode(raw) as Map<String, dynamic>).cast<String, Object?>();
  }

  ({Map<String, List<int>> extraFiles, Map<String, List<int>> genericFiles, int remainingBgZipBudget})?
  assembleDiagnosticsPayload({
    String? description,
    Map<String, Object?>? appInfo,
    required String ringLogsText,
    String? appLogFilePath,
    required String workingDir,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using((arena) {
      ffi.Pointer<Utf8> optStr(String? s) => s == null ? ffi.nullptr : s.toNativeUtf8(allocator: arena);
      return _readAndFree(
        b.assemblePayloadJson(
          optStr(description),
          optStr(appInfo == null ? null : jsonEncode(appInfo)),
          ringLogsText.toNativeUtf8(allocator: arena),
          optStr(appLogFilePath),
          workingDir.toNativeUtf8(allocator: arena),
        ),
      );
    });
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    Map<String, List<int>> decodeFileMap(Map<String, dynamic> b64Map) => {
      for (final MapEntry(:key, :value) in b64Map.entries) key: base64.decode(value as String),
    };
    return (
      extraFiles: decodeFileMap(map['extra_files_b64'] as Map<String, dynamic>),
      genericFiles: decodeFileMap(map['generic_files_b64'] as Map<String, dynamic>),
      remainingBgZipBudget: (map['remaining_bg_zip_budget'] as num).toInt(),
    );
  }

  ({String kind, String? certificatePem, String? message, String? line})? parseCoreStdoutLine(String line) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.parseCoreStdoutLineJson, line);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      kind: map['kind'] as String,
      certificatePem: map['certificate_pem'] as String?,
      message: map['message'] as String?,
      line: map['line'] as String?,
    );
  }

  String? coreCrashHeadline(String output) =>
      _bindings == null ? null : _callStrToStr(_bindings.coreCrashHeadline, output);

  String? coreCrashReasonOf(String crashFilePath, List<String> stderrTail, int startedAtMs) {
    final b = _bindings;
    if (b == null) return null;
    return using(
      (arena) => _readAndFree(
        b.coreCrashReasonOf(
          crashFilePath.toNativeUtf8(allocator: arena),
          stderrTail.join('\n').toNativeUtf8(allocator: arena),
          startedAtMs,
        ),
      ),
    );
  }

  String? windowsAppErrorFor(String xml, int pid) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) => _readAndFree(b.windowsAppErrorFor(xml.toNativeUtf8(allocator: arena), pid)));
  }

  String? describeWindowsExitCode(int exitCode) =>
      _bindings == null ? null : _readAndFree(_bindings.describeWindowsExitCode(exitCode));

  bool? isCoreGoneGrpcError(int grpcCode, String? message) {
    final b = _bindings;
    if (b == null) return null;
    return using((arena) {
      final mPtr = message == null ? ffi.nullptr : message.toNativeUtf8(allocator: arena);
      return _toBool(b.isCoreGoneGrpcError(grpcCode, mPtr));
    });
  }

  String? classifyCoreStartFailure(String message) =>
      _bindings == null ? null : _callStrToStr(_bindings.classifyCoreStartFailure, message);

  // --- Phase 3: Core Diagnostics Export, Config/Log Sanitizer & VLESS X25519 ---

  ({String decryption, String encryption})? generateVlessEncryptionPair() {
    final b = _bindings;
    if (b == null) return null;
    final raw = _readAndFree(b.generateVlessEncryptionJson());
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      decryption: map['decryption'] as String,
      encryption: map['encryption'] as String,
    );
  }

  String? sanitizeConfigJson(String configJson) =>
      _bindings == null ? null : _callStrToStr(_bindings.sanitizeConfigJson, configJson);

  String? sanitizeReportText(String text) =>
      _bindings == null ? null : _callStrToStr(_bindings.sanitizeReportText, text);

  List<({String name, String source, String crashedAt, bool isCrash})>? listCoreCrashReports(
    String workingDir,
  ) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.listCoreCrashReportsJson, workingDir);
    if (raw == null) return null;
    return [
      for (final item in jsonDecode(raw) as List<dynamic>)
        if (item is Map<String, dynamic>)
          (
            name: item['name'] as String,
            source: item['source'] as String,
            crashedAt: (item['crashedAt'] ?? item['crashed_at'] ?? '') as String,
            isCrash: (item['isCrash'] ?? item['is_crash'] ?? false) as bool,
          ),
    ];
  }

  ({String path, bool encrypted})? exportDiagnosticsReport({
    required String workingDir,
    required String tempDir,
    String? destinationPath,
    bool unencrypted = false,
    bool inline = false,
    String? publicKey,
    Map<String, List<int>> extraFiles = const {},
    Map<String, List<int>> genericFiles = const {},
    String? mode,
    String? coreVersion,
    String? coreCommit,
    String? coreState,
  }) {
    final b = _bindings;
    if (b == null) return null;
    Map<String, String> encodeMap(Map<String, List<int>> files) => {
      for (final MapEntry(:key, :value) in files.entries) key: base64.encode(value),
    };
    final reqJson = jsonEncode({
      'workingDir': workingDir,
      'tempDir': tempDir,
      if (destinationPath != null) 'destinationPath': destinationPath,
      'unencrypted': unencrypted,
      'inline': inline,
      if (publicKey != null && publicKey.isNotEmpty) 'publicKey': publicKey,
      'extraFilesB64': encodeMap(extraFiles),
      'genericFilesB64': encodeMap(genericFiles),
      if (mode != null) 'mode': mode,
      if (coreVersion != null) 'coreVersion': coreVersion,
      if (coreCommit != null) 'coreCommit': coreCommit,
      if (coreState != null) 'coreState': coreState,
    });
    final raw = _callStrToStr(b.exportDiagnosticsReportJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    if (map['ok'] != true) {
      throw StateError((map['error'] as String?) ?? 'exportDiagnosticsReport failed');
    }
    return (
      path: map['path'] as String,
      encrypted: map['encrypted'] as bool,
    );
  }

  // --- Phase 4: FetchUrl, Desktop Admin Service Elevation & User System Proxy ---

  ({
    int statusCode,
    List<({String name, String value})> headers,
    Uint8List body,
    int error,
    String errorDetail,
  })?
  fetchUrl({
    required String url,
    String userAgent = '',
    int timeoutMs = 30000,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      'url': url,
      'userAgent': userAgent,
      'timeoutMs': timeoutMs,
    });
    final raw = _callStrToStr(b.fetchUrlJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    if (map['ok'] != true) {
      throw ArgumentError((map['errorDetail'] as String?) ?? 'invalid fetchUrl request');
    }
    final headersList = <({String name, String value})>[
      for (final item in (map['headers'] as List<dynamic>? ?? const []))
        if (item is Map<String, dynamic>)
          (name: item['name'] as String, value: item['value'] as String),
    ];
    final bodyB64 = (map['bodyB64'] as String?) ?? '';
    return (
      statusCode: (map['statusCode'] as num).toInt(),
      headers: headersList,
      body: bodyB64.isEmpty ? Uint8List(0) : base64.decode(bodyB64),
      error: (map['error'] as num).toInt(),
      errorDetail: (map['errorDetail'] as String?) ?? '',
    );
  }

  bool? isAdmin() => _bindings == null ? null : _toBool(_bindings.isAdmin());

  bool? tunAllowed() => _bindings == null ? null : _toBool(_bindings.tunAllowed());

  ({
    int decision,
    List<({String tag, String summary, String detail})> adminNeededProxies,
  })?
  needAdminBgService({
    required String serviceMode,
    required String hiddifySettingsJson,
    required int adminPreference,
    bool privilegeErrorRemembered = false,
    String? cliPath,
    String? buildId,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      'serviceMode': serviceMode,
      'hiddifySettingsJson': hiddifySettingsJson,
      'adminPreference': adminPreference,
      'privilegeErrorRemembered': privilegeErrorRemembered,
      if (cliPath != null) 'cliPath': cliPath,
      if (buildId != null) 'buildId': buildId,
    });
    final raw = _callStrToStr(b.needAdminBgServiceJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    final proxies = <({String tag, String summary, String detail})>[
      for (final item in (map['adminNeededProxies'] as List<dynamic>? ?? const []))
        if (item is Map<String, dynamic>)
          (
            tag: item['tag'] as String,
            summary: item['summary'] as String,
            detail: item['detail'] as String,
          ),
    ];
    return (
      decision: (map['decision'] as num).toInt(),
      adminNeededProxies: proxies,
    );
  }

  List<({int progress, String message, int servicePort})>? ensureAdminBgService({
    required String configContent,
    required String configName,
    required String settingsJson,
    required Map<String, List<int>> resources,
    required bool disableMemoryLimit,
    required int ownerPid,
    required String cliPath,
    required String ownerUser,
    required bool proxyOnly,
    String? buildId,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final resourcesB64 = <String, String>{
      for (final MapEntry(:key, :value) in resources.entries) key: base64.encode(value),
    };
    final reqJson = jsonEncode({
      'configContent': configContent,
      'configName': configName,
      'settingsJson': settingsJson,
      'resourcesB64': resourcesB64,
      'disableMemoryLimit': disableMemoryLimit,
      'ownerPid': ownerPid,
      'cliPath': cliPath,
      'ownerUser': ownerUser,
      'proxyOnly': proxyOnly,
      if (buildId != null) 'buildId': buildId,
    });
    final raw = _callStrToStr(b.ensureAdminBgServiceJson, reqJson);
    if (raw == null) return null;
    return [
      for (final item in (jsonDecode(raw) as List<dynamic>))
        if (item is Map<String, dynamic>)
          (
            progress: (item['progress'] as num).toInt(),
            message: (item['message'] as String?) ?? '',
            servicePort: ((item['service_port'] ?? item['servicePort'] ?? 0) as num).toInt(),
          ),
    ];
  }

  ({bool ok, String message})? setSystemProxyEnabled({
    String? workingDir,
    required bool isEnabled,
    String address = '',
    bool supportSocks = false,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      if (workingDir != null) 'workingDir': workingDir,
      'isEnabled': isEnabled,
      'address': address,
      'supportSocks': supportSocks,
    });
    final raw = _callStrToStr(b.setSystemProxyEnabledJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      ok: (map['ok'] as bool?) ?? false,
      message: (map['message'] as String?) ?? '',
    );
  }

  ({bool available, bool enabled})? getSystemProxyStatus() {
    final b = _bindings;
    if (b == null) return null;
    final raw = _readAndFree(b.getSystemProxyStatusJson());
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      available: (map['available'] as bool?) ?? false,
      enabled: (map['enabled'] as bool?) ?? false,
    );
  }

  // --- Phase 5 (C-ABI v5): Task Cancellation, Update Download, LAN Servers, Health & Pipeline ---

  int? createTask() {
    final b = _bindings;
    if (b == null) return null;
    final id = b.createTask();
    return id == 0 ? null : id;
  }

  bool? cancelTask(int taskId) {
    final b = _bindings;
    if (b == null) return null;
    return _toBool(b.cancelTask(taskId));
  }

  void removeTask(int taskId) {
    final b = _bindings;
    if (b == null || taskId == 0) return;
    b.removeTask(taskId);
  }

  ({int receivedBytes, int totalBytes, bool cancelled, bool finished})? taskProgress(int taskId) {
    final b = _bindings;
    if (b == null || taskId == 0) return null;
    final raw = _readAndFree(b.taskProgressJson(taskId));
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      receivedBytes: (map['receivedBytes'] as num).toInt(),
      totalBytes: (map['totalBytes'] as num).toInt(),
      cancelled: (map['cancelled'] as bool?) ?? false,
      finished: (map['finished'] as bool?) ?? false,
    );
  }

  ({String status, String path, int bytesWritten, String? error})? downloadUpdateVerified({
    required String url,
    required String destinationPath,
    required int expectedSize,
    required String expectedSha256,
    String userAgent = '',
    int proxyPort = 0,
    String proxySecret = '',
    int cancelTaskId = 0,
    int timeoutMs = 30000,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      'url': url,
      'destinationPath': destinationPath,
      'expectedSize': expectedSize,
      'expectedSha256': expectedSha256,
      'userAgent': userAgent,
      'proxyPort': proxyPort,
      'proxySecret': proxySecret,
      'cancelTaskId': cancelTaskId,
      'timeoutMs': timeoutMs,
    });
    final raw = _callStrToStr(b.downloadUpdateVerifiedJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      status: map['status'] as String,
      path: map['path'] as String,
      bytesWritten: (map['bytesWritten'] as num).toInt(),
      error: map['error'] as String?,
    );
  }

  Map<String, dynamic>? healthCheck({
    String target = 'google.com',
    String proxyAddr = '',
    bool checkDns = false,
    bool checkHttp = true,
    bool checkQuic = false,
    String dnsServer = '1.1.1.1:53',
    int timeoutMs = 15000,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      'target': target,
      'proxyAddr': proxyAddr,
      'checkDns': checkDns,
      'checkHttp': checkHttp,
      'checkQuic': checkQuic,
      'dnsServer': dnsServer,
      'timeoutMs': timeoutMs,
    });
    final raw = _callStrToStr(b.healthCheckJson, reqJson);
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  ({int handle, String url, String host, int port, String token})? phoneImportServerStart({
    required String bindIp,
    required Map<String, dynamic> pageText,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using((arena) {
      final ipPtr = bindIp.toNativeUtf8(allocator: arena);
      final txtPtr = jsonEncode(pageText).toNativeUtf8(allocator: arena);
      return _readAndFree(b.phoneImportServerStartJson(ipPtr, txtPtr));
    });
    if (raw == null) return null;
    final outer = jsonDecode(raw) as Map<String, dynamic>;
    if (outer['ok'] != true) {
      throw SocketException((outer['error'] as String?) ?? 'failed to start PhoneImportServer');
    }
    final info = outer['info'] as Map<String, dynamic>;
    return (
      handle: (info['handle'] as num).toInt(),
      url: info['url'] as String,
      host: info['host'] as String,
      port: (info['port'] as num).toInt(),
      token: info['token'] as String,
    );
  }

  ({bool locked, List<({String text, String fromIp, int timestampMs})> entries})? phoneImportServerPoll(
    int handle,
  ) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    final raw = _readAndFree(b.phoneImportServerPollJson(handle));
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    final list = <({String text, String fromIp, int timestampMs})>[];
    for (final item in (map['entries'] as List<dynamic>? ?? const [])) {
      if (item is Map<String, dynamic>) {
        list.add((
          text: item['text'] as String,
          fromIp: item['fromIp'] as String,
          timestampMs: (item['timestampMs'] as num).toInt(),
        ));
      }
    }
    return (locked: (map['locked'] as bool?) ?? false, entries: list);
  }

  bool? phoneImportServerIsLocked(int handle) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    return _toBool(b.phoneImportServerIsLocked(handle));
  }

  bool? phoneImportServerHandled(int handle) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    return _toBool(b.phoneImportServerHandled(handle));
  }

  bool? phoneImportServerClose(int handle) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    return _toBool(b.phoneImportServerClose(handle));
  }

  ({int handle, String url, String host, int port, String token})? lanFileServerStart({
    required String bindIp,
    required String filePath,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final raw = using((arena) {
      final ipPtr = bindIp.toNativeUtf8(allocator: arena);
      final pathPtr = filePath.toNativeUtf8(allocator: arena);
      return _readAndFree(b.lanFileServerStartJson(ipPtr, pathPtr));
    });
    if (raw == null) return null;
    final outer = jsonDecode(raw) as Map<String, dynamic>;
    if (outer['ok'] != true) {
      throw SocketException((outer['error'] as String?) ?? 'failed to start LanFileServer');
    }
    final info = outer['info'] as Map<String, dynamic>;
    return (
      handle: (info['handle'] as num).toInt(),
      url: info['url'] as String,
      host: info['host'] as String,
      port: (info['port'] as num).toInt(),
      token: info['token'] as String,
    );
  }

  List<String>? lanFileServerPoll(int handle) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    final raw = _readAndFree(b.lanFileServerPollJson(handle));
    if (raw == null) return null;
    return [for (final ip in (jsonDecode(raw) as List<dynamic>)) ip as String];
  }

  bool? lanFileServerClose(int handle) {
    final b = _bindings;
    if (b == null || handle == 0) return null;
    return _toBool(b.lanFileServerClose(handle));
  }

  Map<String, dynamic>? fetchAndPrepareRemoteAttempt(Map<String, dynamic> request) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.fetchAndPrepareRemoteAttemptJson, jsonEncode(request));
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  Map<String, dynamic>? finalizeDownloadedRemote(Map<String, dynamic> request) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.finalizeDownloadedRemoteJson, jsonEncode(request));
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  Map<String, dynamic>? prepareLocalProfile(Map<String, dynamic> request) {
    final b = _bindings;
    if (b == null) return null;
    final raw = _callStrToStr(b.prepareLocalProfileJson, jsonEncode(request));
    if (raw == null) return null;
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  ({String status, String? text, String? error})? expandLinksNative({
    required String text,
    bool decodeBase64First = false,
    String userAgent = '',
    int proxyPort = 0,
    String proxySecret = '',
    int timeoutMs = 15000,
    int cancelTaskId = 0,
  }) {
    final b = _bindings;
    if (b == null) return null;
    final reqJson = jsonEncode({
      'text': text,
      'decodeBase64First': decodeBase64First,
      'userAgent': userAgent,
      'proxyPort': proxyPort,
      'proxySecret': proxySecret,
      'timeoutMs': timeoutMs,
      'cancelTaskId': cancelTaskId,
    });
    final raw = _callStrToStr(b.expandLinksNativeJson, reqJson);
    if (raw == null) return null;
    final map = jsonDecode(raw) as Map<String, dynamic>;
    return (
      status: map['status'] as String,
      text: map['text'] as String?,
      error: map['error'] as String?,
    );
  }
}

class _Bindings {
  _Bindings(ffi.DynamicLibrary lib)
    : abiVersion = lib.lookupFunction<_AbiVersionNative, _AbiVersionDart>('hiddify_rust_abi_version'),
      freeString = lib.lookupFunction<_FreeStringNative, _FreeStringDart>('hiddify_rust_free_string'),
      generateIdentityJson = lib.lookupFunction<_NoArgStrNative, _NoArgStrDart>(
        'hiddify_crypto_generate_identity_json',
      ),
      loadOrCreateIdentityJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_crypto_load_or_create_identity_json',
      ),
      isIdentityUsable = lib.lookupFunction<_IsIdentityUsableNative, _IsIdentityUsableDart>(
        'hiddify_crypto_is_identity_usable',
      ),
      verifyPinnedCert = lib.lookupFunction<_VerifyPinnedCertNative, _VerifyPinnedCertDart>(
        'hiddify_crypto_verify_pinned_cert',
      ),
      generateProxySecret = lib.lookupFunction<_SizeToStrNative, _SizeToStrDart>(
        'hiddify_crypto_generate_proxy_secret',
      ),
      isValidSha256Digest = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_crypto_is_valid_sha256_digest',
      ),
      verifyFileSha256 = lib.lookupFunction<_VerifyFileSha256Native, _VerifyFileSha256Dart>(
        'hiddify_crypto_verify_file_sha256',
      ),
      isUrl = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_url'),
      isPort = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_port'),
      isPortOrPortRange = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_net_is_port_or_port_range',
      ),
      isProcessName = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_process_name'),
      isProcessPath = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_process_path'),
      isIpCidr = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_ip_cidr'),
      isDomain = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_domain'),
      isDomainSuffix = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>('hiddify_net_is_domain_suffix'),
      obscureIp = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_net_obscure_ip'),
      sanitizeTag = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_net_sanitize_tag'),
      stripStatusMark = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_net_strip_status_mark'),
      isInvalidProxyType = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_net_is_invalid_proxy_type',
      ),
      splitScannerEndpointJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_split_scanner_endpoint_json',
      ),
      profileProxiesJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_profile_proxies_json',
      ),
      countProxies = lib.lookupFunction<_StrToI64Native, _StrToI64Dart>('hiddify_config_count_proxies'),
      countryOf = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_config_country_of'),
      flagOf = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_config_flag_of'),
      protocolName = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_config_protocol_name'),
      editableConfigJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_editable_config_json',
      ),
      singleProxyConfig = lib.lookupFunction<_SingleProxyConfigNative, _SingleProxyConfigDart>(
        'hiddify_config_single_proxy_config',
      ),
      profileOverride = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>('hiddify_config_profile_override'),
      applyProfileOverride = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_config_apply_profile_override',
      ),
      safeDecodeBase64 = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_profile_safe_decode_base64'),
      decodeBase64Title = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_decode_base64_title',
      ),
      filenameFromContentDisposition = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_filename_from_content_disposition',
      ),
      populateHeadersJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_profile_populate_headers_json',
      ),
      parseSubscriptionInfoJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_parse_subscription_info_json',
      ),
      detectProtocol = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_profile_detect_protocol'),
      isSubscriptionLink = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_profile_is_subscription_link',
      ),
      parseLinkJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_profile_parse_link_json'),
      parseSimpleLinkJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_parse_simple_link_json',
      ),
      parseDeepLinkJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_parse_deep_link_json',
      ),
      generateSubShareLink = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_profile_generate_sub_share_link',
      ),
      decodeTextBytes = lib.lookupFunction<_BytesToStrNative, _BytesToStrDart>(
        'hiddify_profile_decode_text_bytes',
      ),
      readZipText = lib.lookupFunction<_BytesToStrNative, _BytesToStrDart>('hiddify_profile_read_zip_text'),
      telegramPayload = lib.lookupFunction<_TelegramPayloadNative, _TelegramPayloadDart>(
        'hiddify_diagnostics_telegram_payload',
      ),
      windowsVersion = lib.lookupFunction<_StrToStrNative, _StrToStrDart>('hiddify_diagnostics_windows_version'),
      linuxDistroJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_linux_distro_json',
      ),
      crashExcerptJson = lib.lookupFunction<_CrashExcerptNative, _CrashExcerptDart>(
        'hiddify_diagnostics_crash_excerpt_json',
      ),
      extractChangelog = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_extract_changelog',
      ),
      parseReleaseTagJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_parse_release_tag_json',
      ),
      generateNumericPin = lib.lookupFunction<_SizeToStrNative, _SizeToStrDart>(
        'hiddify_crypto_generate_numeric_pin',
      ),
      generateUrlToken = lib.lookupFunction<_SizeToStrNative, _SizeToStrDart>(
        'hiddify_crypto_generate_url_token',
      ),
      generateAlphanumericSecret = lib.lookupFunction<_SizeToStrNative, _SizeToStrDart>(
        'hiddify_crypto_generate_alphanumeric_secret',
      ),
      verifySecretPath = lib.lookupFunction<_Str2ToBoolNative, _Str2ToBoolDart>(
        'hiddify_crypto_verify_secret_path',
      ),
      isLocalNetworkBytes = lib.lookupFunction<_BytesToBoolNative, _BytesToBoolDart>(
        'hiddify_net_is_local_network_bytes',
      ),
      isAllowedHostHeader = lib.lookupFunction<_Str2ToBoolNative, _Str2ToBoolDart>(
        'hiddify_net_is_allowed_host_header',
      ),
      parsePhoneImportFormJson = lib.lookupFunction<_FormParseNative, _FormParseDart>(
        'hiddify_net_parse_phone_import_form_json',
      ),
      parseOptionalRangeJson = lib.lookupFunction<_CrashExcerptNative, _CrashExcerptDart>(
        'hiddify_config_parse_optional_range_json',
      ),
      formatOptionalRange = lib.lookupFunction<_FormatRangeNative, _FormatRangeDart>(
        'hiddify_config_format_optional_range',
      ),
      extraSecurityMode = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_extra_security_mode',
      ),
      keepTopHeader = lib.lookupFunction<_StrToI64Native, _StrToI64Dart>(
        'hiddify_config_keep_top_header',
      ),
      fragmentTricksJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_fragment_tricks_json',
      ),
      keepOwnDnsJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_config_keep_own_dns_json',
      ),
      mergeOptionsWithOverrideJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_config_merge_options_with_override_json',
      ),
      builtinRulePbJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_config_builtin_rule_pb_json',
      ),
      builtinOfRuleName = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_builtin_of_rule_name',
      ),
      withBuiltinsPbJson = lib.lookupFunction<_Str3ToStrNative, _Str3ToStrDart>(
        'hiddify_config_with_builtins_pb_json',
      ),
      duplicateRulePbJson = lib.lookupFunction<_DupRuleNative, _DupRuleDart>(
        'hiddify_config_duplicate_rule_pb_json',
      ),
      downloadProblemOfStatus = lib.lookupFunction<_I32ToI32Native, _I32ToI32Dart>(
        'hiddify_profile_download_problem_of_status',
      ),
      downloadProblemOfCore = lib.lookupFunction<_I32PairToI32Native, _I32PairToI32Dart>(
        'hiddify_profile_download_problem_of_core',
      ),
      downloadProblemClassifyDio = lib.lookupFunction<_ClassifyDioNative, _ClassifyDioDart>(
        'hiddify_profile_download_problem_classify_dio',
      ),
      downloadProblemMostTelling = lib.lookupFunction<_MostTellingNative, _MostTellingDart>(
        'hiddify_profile_download_problem_most_telling',
      ),
      downloadProblemMetaJson = lib.lookupFunction<_I32PairToStrNative, _I32PairToStrDart>(
        'hiddify_profile_download_problem_meta_json',
      ),
      flattenHeadersJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_flatten_headers_json',
      ),
      planLinkExpansionJson = lib.lookupFunction<_CrashExcerptNative, _CrashExcerptDart>(
        'hiddify_profile_plan_link_expansion_json',
      ),
      mergeExpandedLinks = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_profile_merge_expanded_links',
      ),
      resolveProfileName = lib.lookupFunction<_ResolveNameNative, _ResolveNameDart>(
        'hiddify_profile_resolve_profile_name',
      ),
      parseUpdateIntervalHours = lib.lookupFunction<_UpdateIntervalNative, _UpdateIntervalDart>(
        'hiddify_profile_parse_update_interval_hours',
      ),
      openLogFile = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_diagnostics_open_log_file',
      ),
      isLogFileOpen = lib.lookupFunction<_NoArgBoolNative, _NoArgBoolDart>(
        'hiddify_diagnostics_is_log_file_open',
      ),
      writeLogLine = lib.lookupFunction<ffi.Void Function(ffi.Pointer<Utf8>), void Function(ffi.Pointer<Utf8>)>(
        'hiddify_diagnostics_write_log_line',
      ),
      closeLogFile = lib.lookupFunction<_NoArgVoidNative, _NoArgVoidDart>(
        'hiddify_diagnostics_close_log_file',
      ),
      writeTextToFile = lib.lookupFunction<_Str2ToBoolNative, _Str2ToBoolDart>(
        'hiddify_diagnostics_write_text_to_file',
      ),
      originOfStack = lib.lookupFunction<_Str3ToStrNative, _Str3ToStrDart>(
        'hiddify_diagnostics_origin_of_stack',
      ),
      ownStackFrames = lib.lookupFunction<_OwnFramesNative, _OwnFramesDart>(
        'hiddify_diagnostics_own_stack_frames',
      ),
      openAppCrashStore = lib.lookupFunction<_StrToBoolNative, _StrToBoolDart>(
        'hiddify_diagnostics_open_app_crash_store',
      ),
      recordAppCrash = lib.lookupFunction<_Str2VoidNative, _Str2VoidDart>(
        'hiddify_diagnostics_record_app_crash',
      ),
      listAppCrashFilesJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_list_app_crash_files_json',
      ),
      listFatalAppCrashReportsJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_list_fatal_app_crash_reports_json',
      ),
      scrubPreferencesJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_scrub_preferences_json',
      ),
      assemblePayloadJson = lib.lookupFunction<_AssembleDiagNative, _AssembleDiagDart>(
        'hiddify_diagnostics_assemble_payload_json',
      ),
      parseCoreStdoutLineJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_parse_core_stdout_line_json',
      ),
      coreCrashHeadline = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_core_crash_headline',
      ),
      coreCrashReasonOf = lib.lookupFunction<_CrashReasonOfNative, _CrashReasonOfDart>(
        'hiddify_diagnostics_core_crash_reason_of',
      ),
      windowsAppErrorFor = lib.lookupFunction<_DupRuleNative, _DupRuleDart>(
        'hiddify_diagnostics_windows_app_error_for',
      ),
      describeWindowsExitCode = lib.lookupFunction<_I64ToStrNative, _I64ToStrDart>(
        'hiddify_diagnostics_describe_windows_exit_code',
      ),
      isCoreGoneGrpcError = lib.lookupFunction<_CoreGoneNative, _CoreGoneDart>(
        'hiddify_diagnostics_is_core_gone_grpc_error',
      ),
      classifyCoreStartFailure = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_classify_core_start_failure',
      ),
      generateVlessEncryptionJson = lib.lookupFunction<_NoArgStrNative, _NoArgStrDart>(
        'hiddify_crypto_generate_vless_encryption_json',
      ),
      sanitizeConfigJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_config_sanitize_json',
      ),
      sanitizeReportText = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_sanitize_report_text',
      ),
      listCoreCrashReportsJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_list_core_crash_reports_json',
      ),
      exportDiagnosticsReportJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_diagnostics_export_report_json',
      ),
      fetchUrlJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_fetch_url_json',
      ),
      isAdmin = lib.lookupFunction<_NoArgBoolNative, _NoArgBoolDart>(
        'hiddify_net_is_admin',
      ),
      tunAllowed = lib.lookupFunction<_NoArgBoolNative, _NoArgBoolDart>(
        'hiddify_net_tun_allowed',
      ),
      needAdminBgServiceJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_need_admin_bg_service_json',
      ),
      ensureAdminBgServiceJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_ensure_admin_bg_service_json',
      ),
      setSystemProxyEnabledJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_set_system_proxy_enabled_json',
      ),
      getSystemProxyStatusJson = lib.lookupFunction<_NoArgStrNative, _NoArgStrDart>(
        'hiddify_net_get_system_proxy_status_json',
      ),
      createTask = lib.lookupFunction<_NoArgU64Native, _NoArgU64Dart>(
        'hiddify_net_create_task',
      ),
      cancelTask = lib.lookupFunction<_U64ToBoolNative, _U64ToBoolDart>(
        'hiddify_net_cancel_task',
      ),
      removeTask = lib.lookupFunction<_U64VoidNative, _U64VoidDart>(
        'hiddify_net_remove_task',
      ),
      taskProgressJson = lib.lookupFunction<_U64ToStrNative, _U64ToStrDart>(
        'hiddify_net_task_progress_json',
      ),
      downloadUpdateVerifiedJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_download_update_verified_json',
      ),
      healthCheckJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_net_health_check_json',
      ),
      phoneImportServerStartJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_net_phone_import_server_start_json',
      ),
      phoneImportServerPollJson = lib.lookupFunction<_U64ToStrNative, _U64ToStrDart>(
        'hiddify_net_phone_import_server_poll_json',
      ),
      phoneImportServerIsLocked = lib.lookupFunction<_U64ToBoolNative, _U64ToBoolDart>(
        'hiddify_net_phone_import_server_is_locked',
      ),
      phoneImportServerHandled = lib.lookupFunction<_U64ToBoolNative, _U64ToBoolDart>(
        'hiddify_net_phone_import_server_handled',
      ),
      phoneImportServerClose = lib.lookupFunction<_U64ToBoolNative, _U64ToBoolDart>(
        'hiddify_net_phone_import_server_close',
      ),
      lanFileServerStartJson = lib.lookupFunction<_Str2ToStrNative, _Str2ToStrDart>(
        'hiddify_net_lan_file_server_start_json',
      ),
      lanFileServerPollJson = lib.lookupFunction<_U64ToStrNative, _U64ToStrDart>(
        'hiddify_net_lan_file_server_poll_json',
      ),
      lanFileServerClose = lib.lookupFunction<_U64ToBoolNative, _U64ToBoolDart>(
        'hiddify_net_lan_file_server_close',
      ),
      fetchAndPrepareRemoteAttemptJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_fetch_and_prepare_remote_attempt_json',
      ),
      finalizeDownloadedRemoteJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_finalize_downloaded_remote_json',
      ),
      prepareLocalProfileJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_prepare_local_json',
      ),
      expandLinksNativeJson = lib.lookupFunction<_StrToStrNative, _StrToStrDart>(
        'hiddify_profile_expand_links_native_json',
      );

  final _AbiVersionDart abiVersion;
  final _FreeStringDart freeString;
  final _NoArgStrDart generateIdentityJson;
  final _StrToStrDart loadOrCreateIdentityJson;
  final _IsIdentityUsableDart isIdentityUsable;
  final _VerifyPinnedCertDart verifyPinnedCert;
  final _SizeToStrDart generateProxySecret;
  final _StrToBoolDart isValidSha256Digest;
  final _VerifyFileSha256Dart verifyFileSha256;

  final _StrToBoolDart isUrl;
  final _StrToBoolDart isPort;
  final _StrToBoolDart isPortOrPortRange;
  final _StrToBoolDart isProcessName;
  final _StrToBoolDart isProcessPath;
  final _StrToBoolDart isIpCidr;
  final _StrToBoolDart isDomain;
  final _StrToBoolDart isDomainSuffix;
  final _StrToStrDart obscureIp;
  final _StrToStrDart sanitizeTag;
  final _StrToStrDart stripStatusMark;
  final _StrToBoolDart isInvalidProxyType;
  final _StrToStrDart splitScannerEndpointJson;

  final _StrToStrDart profileProxiesJson;
  final _StrToI64Dart countProxies;
  final _StrToStrDart countryOf;
  final _StrToStrDart flagOf;
  final _StrToStrDart protocolName;
  final _StrToStrDart editableConfigJson;
  final _SingleProxyConfigDart singleProxyConfig;
  final _Str2ToStrDart profileOverride;
  final _Str2ToStrDart applyProfileOverride;

  final _StrToStrDart safeDecodeBase64;
  final _StrToStrDart decodeBase64Title;
  final _StrToStrDart filenameFromContentDisposition;
  final _Str2ToStrDart populateHeadersJson;
  final _StrToStrDart parseSubscriptionInfoJson;
  final _StrToStrDart detectProtocol;
  final _StrToBoolDart isSubscriptionLink;
  final _StrToStrDart parseLinkJson;
  final _StrToStrDart parseSimpleLinkJson;
  final _StrToStrDart parseDeepLinkJson;
  final _Str2ToStrDart generateSubShareLink;
  final _BytesToStrDart decodeTextBytes;
  final _BytesToStrDart readZipText;

  final _TelegramPayloadDart telegramPayload;
  final _StrToStrDart windowsVersion;
  final _StrToStrDart linuxDistroJson;
  final _CrashExcerptDart crashExcerptJson;
  final _StrToStrDart extractChangelog;
  final _StrToStrDart parseReleaseTagJson;

  final _SizeToStrDart generateNumericPin;
  final _SizeToStrDart generateUrlToken;
  final _SizeToStrDart generateAlphanumericSecret;
  final _Str2ToBoolDart verifySecretPath;

  final _BytesToBoolDart isLocalNetworkBytes;
  final _Str2ToBoolDart isAllowedHostHeader;
  final _FormParseDart parsePhoneImportFormJson;

  final _CrashExcerptDart parseOptionalRangeJson;
  final _FormatRangeDart formatOptionalRange;
  final _StrToStrDart extraSecurityMode;
  final _StrToI64Dart keepTopHeader;
  final _StrToStrDart fragmentTricksJson;
  final _Str2ToStrDart keepOwnDnsJson;
  final _Str2ToStrDart mergeOptionsWithOverrideJson;
  final _Str2ToStrDart builtinRulePbJson;
  final _StrToStrDart builtinOfRuleName;
  final _Str3ToStrDart withBuiltinsPbJson;
  final _DupRuleDart duplicateRulePbJson;

  final _I32ToI32Dart downloadProblemOfStatus;
  final _I32PairToI32Dart downloadProblemOfCore;
  final _ClassifyDioDart downloadProblemClassifyDio;
  final _MostTellingDart downloadProblemMostTelling;
  final _I32PairToStrDart downloadProblemMetaJson;
  final _StrToStrDart flattenHeadersJson;
  final _CrashExcerptDart planLinkExpansionJson;
  final _Str2ToStrDart mergeExpandedLinks;
  final _ResolveNameDart resolveProfileName;
  final _UpdateIntervalDart parseUpdateIntervalHours;

  final _StrToBoolDart openLogFile;
  final _NoArgBoolDart isLogFileOpen;
  final void Function(ffi.Pointer<Utf8>) writeLogLine;
  final _NoArgVoidDart closeLogFile;
  final _Str2ToBoolDart writeTextToFile;
  final _Str3ToStrDart originOfStack;
  final _OwnFramesDart ownStackFrames;
  final _StrToBoolDart openAppCrashStore;
  final _Str2VoidDart recordAppCrash;
  final _StrToStrDart listAppCrashFilesJson;
  final _StrToStrDart listFatalAppCrashReportsJson;
  final _StrToStrDart scrubPreferencesJson;
  final _AssembleDiagDart assemblePayloadJson;
  final _StrToStrDart parseCoreStdoutLineJson;
  final _StrToStrDart coreCrashHeadline;
  final _CrashReasonOfDart coreCrashReasonOf;
  final _DupRuleDart windowsAppErrorFor;
  final _I64ToStrDart describeWindowsExitCode;
  final _CoreGoneDart isCoreGoneGrpcError;
  final _StrToStrDart classifyCoreStartFailure;
  final _NoArgStrDart generateVlessEncryptionJson;
  final _StrToStrDart sanitizeConfigJson;
  final _StrToStrDart sanitizeReportText;
  final _StrToStrDart listCoreCrashReportsJson;
  final _StrToStrDart exportDiagnosticsReportJson;
  final _StrToStrDart fetchUrlJson;
  final _NoArgBoolDart isAdmin;
  final _NoArgBoolDart tunAllowed;
  final _StrToStrDart needAdminBgServiceJson;
  final _StrToStrDart ensureAdminBgServiceJson;
  final _StrToStrDart setSystemProxyEnabledJson;
  final _NoArgStrDart getSystemProxyStatusJson;
  final _NoArgU64Dart createTask;
  final _U64ToBoolDart cancelTask;
  final _U64VoidDart removeTask;
  final _U64ToStrDart taskProgressJson;
  final _StrToStrDart downloadUpdateVerifiedJson;
  final _StrToStrDart healthCheckJson;
  final _Str2ToStrDart phoneImportServerStartJson;
  final _U64ToStrDart phoneImportServerPollJson;
  final _U64ToBoolDart phoneImportServerIsLocked;
  final _U64ToBoolDart phoneImportServerHandled;
  final _U64ToBoolDart phoneImportServerClose;
  final _Str2ToStrDart lanFileServerStartJson;
  final _U64ToStrDart lanFileServerPollJson;
  final _U64ToBoolDart lanFileServerClose;
  final _StrToStrDart fetchAndPrepareRemoteAttemptJson;
  final _StrToStrDart finalizeDownloadedRemoteJson;
  final _StrToStrDart prepareLocalProfileJson;
  final _StrToStrDart expandLinksNativeJson;
}
