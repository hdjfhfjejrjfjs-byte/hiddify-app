import 'package:hiddify/core/rust/hiddify_rust.dart';

/// The app's own crashes, one file each in `app_crashes/` in the working directory, so the next
/// run can offer them in a report: the in-memory log of the run that crashed is gone by then.
///
/// The Android side writes its crashes here too (AppCrashRecorder.kt), with the same names:
/// `<UTC time to the millisecond>-<kind>.txt`, kind `dart`, `android`, `native` or `anr`.
const appCrashDirectory = 'app_crashes';

/// Called once the working directory is known; an error before that is in the log only.
void openAppCrashStore(String workingDir) {
  hiddifyRust.require(hiddifyRust.openAppCrashStore(workingDir), 'openAppCrashStore');
}

/// Writes [text] at once, without waiting: the process may be about to end.
void recordAppCrash(String kind, String text) {
  hiddifyRust.ensureAvailable('recordAppCrash');
  hiddifyRust.recordAppCrash(kind, text);
}
