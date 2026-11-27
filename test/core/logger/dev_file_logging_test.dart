import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:hiddify/core/logger/log_level_compat.dart';
import 'package:hiddify/core/logger/logger_setup.dart';
import 'package:hiddify/core/logger/sinks/file_sink_io.dart';
import 'package:logging/logging.dart';

void main() {
  late Directory dir;
  late File file;
  late Logger testLogger;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('hiddify_dev_file_logging');
    file = File('${dir.path}/app.log');
    Logger.root.level = Level.ALL;
    logSetFileLevel(Level.SEVERE);
    testLogger = Logger('dev_log_test');
  });

  tearDown(() async {
    logFinish(keepFile: false);
    await closeLogFile();
    logSetFileLevel(Level.SEVERE);
    if (dir.existsSync()) {
      await dir.delete(recursive: true);
    }
  });

  test('default Level.SEVERE writes only SEVERE and above to the log file', () {
    expect(fileLevel, Level.SEVERE);
    logAddFile(file.path);

    testLogger.trace('trace ignored');
    testLogger.debug('debug ignored');
    testLogger.info('info ignored');
    testLogger.warning('warning ignored');
    testLogger.severe('severe recorded');

    final lines = file.readAsLinesSync();
    expect(lines, hasLength(1));
    expect(lines.single, contains('[ERROR] [dev_log_test] severe recorded'));
  });

  test('logSetFileLevel(Level.FINE) writes FINE, INFO, WARNING, and SEVERE records', () {
    logSetFileLevel(Level.FINE);
    expect(fileLevel, Level.FINE);
    logAddFile(file.path);

    testLogger.trace('trace still filtered');
    testLogger.debug('debug recorded');
    testLogger.info('info recorded');
    testLogger.warning('warning recorded');
    testLogger.severe('severe recorded');

    final lines = file.readAsLinesSync();
    expect(lines, hasLength(4));
    expect(lines[0], contains('[DEBUG] [dev_log_test] debug recorded'));
    expect(lines[1], contains('[INFO] [dev_log_test] info recorded'));
    expect(lines[2], contains('[WARN] [dev_log_test] warning recorded'));
    expect(lines[3], contains('[ERROR] [dev_log_test] severe recorded'));
  });

  test('raising logSetFileLevel back to Level.SEVERE restores error-only filtering', () {
    logSetFileLevel(Level.FINE);
    logAddFile(file.path);

    testLogger.info('dev info message');
    logSetFileLevel(Level.SEVERE);
    testLogger.info('prod info dropped');
    testLogger.severe('prod severe kept');

    final lines = file.readAsLinesSync();
    expect(lines, hasLength(2));
    expect(lines[0], contains('[INFO] [dev_log_test] dev info message'));
    expect(lines[1], contains('[ERROR] [dev_log_test] prod severe kept'));
  });
}
