import 'dart:typed_data';

/// Stub implementation used on platforms without `dart:ffi` (e.g. Flutter Web).
class HiddifyRustNative {
  HiddifyRustNative._();

  static final HiddifyRustNative instance = HiddifyRustNative._();

  bool get isAvailable => false;

  ({String certificatePem, String privateKeyPem})? generateIdentity() => null;
  ({String certificatePem, String privateKeyPem})? loadOrCreateIdentity(String directoryPath) => null;
  bool? isIdentityUsable(String certificatePem, String privateKeyPem, int renewBeforeSecs) => null;
  bool? verifyPinnedCert(Uint8List presentedDer, String presentedPem, String expectedServerPem, String host) => null;
  String? generateProxySecret(int length) => null;
  bool? isValidSha256Digest(String digest) => null;
  bool? verifyFileSha256(String filePath, int expectedSize, String expectedDigest) => null;

  bool? isUrl(String input) => null;
  bool? isPort(String input) => null;
  bool? isPortOrPortRange(String input) => null;
  bool? isProcessName(String input) => null;
  bool? isProcessPath(String input) => null;
  bool? isIpCidr(String input) => null;
  bool? isDomain(String input) => null;
  bool? isDomainSuffix(String input) => null;
  String? obscureIp(String ip) => null;
  String? sanitizeTag(String tag) => null;
  String? stripStatusMark(String proxyType) => null;
  bool? isInvalidProxyType(String typeWithoutStatus) => null;
  ({String before, String endpoint, String after})? splitScannerEndpoint(String proxyType) => null;

  List<({String tag, String type})>? profileProxies(String configJson) => null;
  int? countProxies(String configJson) => null;
  String? countryOf(String tag) => null;
  String? flagOf(String code) => null;
  String? protocolName(String proxyType) => null;
  ({String text, bool isJson})? editableConfig(String content) => null;
  String? singleProxyConfig(String tag, {String host = '', int port = 0, String? raw, String? generated}) => null;
  String? profileOverride(Map<String, dynamic>? populatedHeaders, Map<String, dynamic>? userOverride) => null;
  Map<String, dynamic>? applyProfileOverride(Map<String, dynamic> main, String? profileOverride) => null;

  String? safeDecodeBase64(String input) => null;
  String? decodeBase64Title(String value) => null;
  String? filenameFromContentDisposition(String header) => null;
  Map<String, dynamic>? populateHeaders(String content, Map<String, dynamic>? remoteHeaders) => null;
  ({int upload, int download, int total, int expireSecs})? parseSubscriptionInfo(String subInfoStr) => null;
  String? detectProtocol(String content) => null;
  bool? isSubscriptionLink(String line) => null;
  ({String url, String name})? parseLink(String link) => null;
  ({String url, String name})? parseSimpleLink(String link) => null;
  ({String url, String name})? parseDeepLink(String link) => null;
  String? generateSubShareLink(String url, String? name) => null;
  String? decodeTextBytes(Uint8List bytes) => null;
  String? readZipText(Uint8List bytes) => null;

  String? telegramPayload({
    required String os,
    required String ov,
    required String v,
    required String t,
    String? pm,
    String? l,
    String? a,
  }) => null;
  String? windowsVersion(String osVersion) => null;
  (String, String)? linuxDistro(String osRelease) => null;
  ({String headline, String text})? crashExcerpt(String content, {required bool findGoMarker}) => null;
  String? extractChangelog(String body) => null;
  ({String version, String suffix, String buildNumber})? parseReleaseTag(String tagName) => null;

  // --- Phase 2 ---
  String? generateNumericPin(int length) => null;
  String? generateUrlToken(int byteLen) => null;
  String? generateAlphanumericSecret(int length) => null;
  bool? verifySecretPath(String path, String token) => null;

  bool? isLocalNetworkBytes(Uint8List rawIp) => null;
  bool? isAllowedHostHeader(String? hostHeader, String expectedAuthority) => null;
  ({String status, String? text})? parsePhoneImportForm(Uint8List body, int maxTextBytes) => null;

  ({int? min, int? max})? parseOptionalRange(String input, {required bool allowEmpty}) => null;
  String? formatOptionalRange(int? min, int? max) => null;
  String? extraSecurityMode(String? value) => null;
  int? keepTopHeader(String? value) => null;
  Map<String, dynamic>? fragmentTricks(String? value) => null;
  Map<String, dynamic>? keepOwnDnsBehindExtraSecurity(Map<String, dynamic> own, Map<String, dynamic> merged) => null;
  Map<String, dynamic>? mergeOptionsWithProfileOverride(Map<String, dynamic> own, String? profileOverride) => null;
  String? builtinRulePbJson(String shortName, String platform) => null;
  String? builtinOfRuleName(String ruleName) => null;
  String? withBuiltinsPbJson(String savedPbJson, String region, String platform) => null;
  ({String pbJson, int order})? duplicateRulePbJson(String currentPbJson, int listOrder) => null;

  int? downloadProblemOfStatus(int code) => null;
  int? downloadProblemOfCore(int fetchError, int statusCode) => null;
  int? downloadProblemClassifyDio({
    int? statusCode,
    required String dioType,
    required bool isCertExc,
    required bool isTlsExc,
    required String text,
  }) => null;
  int? downloadProblemMostTelling(List<int> problemIndices) => null;
  ({bool mayPass, String code, int? failedStop, int side})? downloadProblemMeta(int problemIndex, int? statusCode) =>
      null;
  Map<String, dynamic>? flattenHttpHeaders(Map<String, List<String>> headers) => null;
  ({String decodedText, List<({int index, String url})> targets})? planLinkExpansion(
    String rawText, {
    required bool decodeBase64First,
  }) => null;
  String? mergeExpandedLinks(String decodedText, List<(int, String)> replacements) => null;
  String? resolveProfileName({
    String? overrideName,
    String? profileTitleHeader,
    String? contentDispositionHeader,
    String? remoteUrl,
    String? existingLocalName,
    required String tempFilePath,
  }) => null;
  int? parseUpdateIntervalHours({
    String? headerValue,
    int? overrideHours,
    required bool isAutoUpdateDisable,
  }) => null;

  bool? openLogFile(String path) => null;
  bool? isLogFileOpen() => null;
  void writeLogLine(String line) {}
  void closeLogFile() {}
  bool? writeTextToFile(String path, String content) => null;
  String? originOfStack(String stack, String ownPrefix, String loggerInternals) => null;
  String? ownStackFrames(String stack, String ownPrefix, String loggerInternals, int maxFrames) => null;
  bool? openAppCrashStore(String workingDir) => null;
  void recordAppCrash(String kind, String text) {}
  List<String>? listAppCrashFiles(String workingDir) => null;
  List<({String name, String source, String crashedAt})>? listFatalAppCrashReports(String workingDir) => null;
  Map<String, Object?>? scrubPreferences(Map<String, Object?> prefs) => null;
  ({Map<String, List<int>> extraFiles, Map<String, List<int>> genericFiles, int remainingBgZipBudget})?
  assembleDiagnosticsPayload({
    String? description,
    Map<String, Object?>? appInfo,
    required String ringLogsText,
    String? appLogFilePath,
    required String workingDir,
  }) => null;
  ({String kind, String? certificatePem, String? message, String? line})? parseCoreStdoutLine(String line) => null;
  String? coreCrashHeadline(String output) => null;
  String? coreCrashReasonOf(String crashFilePath, List<String> stderrTail, int startedAtMs) => null;
  String? windowsAppErrorFor(String xml, int pid) => null;
  String? describeWindowsExitCode(int exitCode) => null;
  bool? isCoreGoneGrpcError(int grpcCode, String? message) => null;
  String? classifyCoreStartFailure(String message) => null;

  ({String decryption, String encryption})? generateVlessEncryptionPair() => null;
  String? sanitizeConfigJson(String configJson) => null;
  String? sanitizeReportText(String text) => null;
  List<({String name, String source, String crashedAt, bool isCrash})>? listCoreCrashReports(String workingDir) => null;
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
  }) => null;

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
  }) => null;

  bool? isAdmin() => null;
  bool? tunAllowed() => null;

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
  }) => null;

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
  }) => null;

  ({bool ok, String message})? setSystemProxyEnabled({
    String? workingDir,
    required bool isEnabled,
    String address = '',
    bool supportSocks = false,
  }) => null;

  ({bool available, bool enabled})? getSystemProxyStatus() => null;

  int? createTask() => null;
  bool? cancelTask(int taskId) => null;
  void removeTask(int taskId) {}
  ({int receivedBytes, int totalBytes, bool cancelled, bool finished})? taskProgress(int taskId) => null;

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
  }) => null;

  Map<String, dynamic>? healthCheck({
    String target = 'google.com',
    String proxyAddr = '',
    bool checkDns = false,
    bool checkHttp = true,
    bool checkQuic = false,
    String dnsServer = '1.1.1.1:53',
    int timeoutMs = 15000,
  }) => null;

  ({int handle, String url, String host, int port, String token})? phoneImportServerStart({
    required String bindIp,
    required Map<String, dynamic> pageText,
  }) => null;
  ({bool locked, List<({String text, String fromIp, int timestampMs})> entries})? phoneImportServerPoll(int handle) =>
      null;
  bool? phoneImportServerIsLocked(int handle) => null;
  bool? phoneImportServerHandled(int handle) => null;
  bool? phoneImportServerClose(int handle) => null;

  ({int handle, String url, String host, int port, String token})? lanFileServerStart({
    required String bindIp,
    required String filePath,
  }) => null;
  List<String>? lanFileServerPoll(int handle) => null;
  bool? lanFileServerClose(int handle) => null;

  Map<String, dynamic>? fetchAndPrepareRemoteAttempt(Map<String, dynamic> request) => null;
  Map<String, dynamic>? finalizeDownloadedRemote(Map<String, dynamic> request) => null;
  Map<String, dynamic>? prepareLocalProfile(Map<String, dynamic> request) => null;
  ({String status, String? text, String? error})? expandLinksNative({
    required String text,
    bool decodeBase64First = false,
    String userAgent = '',
    int proxyPort = 0,
    String proxySecret = '',
    int timeoutMs = 15000,
    int cancelTaskId = 0,
  }) => null;
}
