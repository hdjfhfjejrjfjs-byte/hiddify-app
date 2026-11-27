import UIKit
import Flutter
import HiddifyCore
import Sentry

@main
@objc class AppDelegate: FlutterAppDelegate {
    
    override func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        setupFileManager()
        SharedLogMirror.shared.syncNow()
        registerHandlers()
        GeneratedPluginRegistrant.register(with: self)
        return super.application(application, didFinishLaunchingWithOptions: launchOptions)
    }

    override func applicationDidBecomeActive(_ application: UIApplication) {
        super.applicationDidBecomeActive(application)
        SharedLogMirror.shared.startForegroundSync()
    }

    override func applicationWillResignActive(_ application: UIApplication) {
        SharedLogMirror.shared.stopForegroundSync(flush: true)
        super.applicationWillResignActive(application)
    }

    override func applicationDidEnterBackground(_ application: UIApplication) {
        SharedLogMirror.shared.stopForegroundSync(flush: true)
        super.applicationDidEnterBackground(application)
    }
    
    func setupFileManager() {
        FilePath.prepareWorkingDirectory()
        FileManager.default.changeCurrentDirectoryPath(FilePath.sharedDirectory.path)
    }
    
    func registerHandlers() {
        // The registrar is missing when iOS launches the app without a Flutter engine (a launch in
        // the background, for one): skip the handlers instead of crashing the launch.
        func register(_ name: String, _ register: (FlutterPluginRegistrar) -> Void) {
            guard let registrar = self.registrar(forPlugin: name) else {
                NSLog("hiddify: no Flutter registrar for \(name), handler not registered")
                return
            }
            register(registrar)
        }
        register(MethodHandler.name) { MethodHandler.register(with: $0) }
        register(PlatformMethodHandler.name) { PlatformMethodHandler.register(with: $0) }
        register(FileMethodHandler.name) { FileMethodHandler.register(with: $0) }
        register(StatusEventHandler.name) { StatusEventHandler.register(with: $0) }
        register(AlertsEventHandler.name) { AlertsEventHandler.register(with: $0) }
//        LogsEventHandler.register(with: self.registrar(forPlugin: LogsEventHandler.name)!)
//        GroupsEventHandler.register(with: self.registrar(forPlugin: GroupsEventHandler.name)!)
//        ActiveGroupsEventHandler.register(with: self.registrar(forPlugin: ActiveGroupsEventHandler.name)!)
//        StatsEventHandler.register(with: self.registrar(forPlugin: StatsEventHandler.name)!)
    }
}

/// Mirrors non-empty logs from the hidden App Group working directory into `<AppSandbox>/Documents/logs/`
/// so users can inspect live logs directly in the iOS Files app ("On My iPhone -> Hiddify -> logs") or Finder.
final class SharedLogMirror {
    static let shared = SharedLogMirror()

    private struct FileStamp: Equatable {
        let path: String
        let size: UInt64
        let modificationDate: Date?
    }

    private let queue = DispatchQueue(label: "com.hiddify.ios.log-mirror", qos: .utility)
    private var timer: DispatchSourceTimer?
    private var lastSynced: [String: FileStamp] = [:]

    private init() {}

    func syncNow() {
        queue.async { [weak self] in
            self?.syncAllOnQueue()
        }
    }

    func startForegroundSync() {
        queue.async { [weak self] in
            guard let self else { return }
            self.syncAllOnQueue()
            guard self.timer == nil else { return }
            let timer = DispatchSource.makeTimerSource(queue: self.queue)
            timer.schedule(deadline: .now() + .seconds(3), repeating: .seconds(3), leeway: .milliseconds(500))
            timer.setEventHandler { [weak self] in
                self?.syncAllOnQueue()
            }
            self.timer = timer
            timer.resume()
        }
    }

    func stopForegroundSync(flush: Bool) {
        queue.async { [weak self] in
            guard let self else { return }
            self.timer?.cancel()
            self.timer = nil
            if flush {
                self.syncAllOnQueue()
            }
        }
    }

    private func syncAllOnQueue() {
        let fileManager = FileManager.default
        guard let documentsDir = fileManager.urls(for: .documentDirectory, in: .userDomainMask).first else {
            return
        }
        let logsDir = documentsDir.appendingPathComponent("logs", isDirectory: true)
        let workingDir = FilePath.workingDirectory

        let fixedTargets: [(source: URL, destinationName: String)] = [
            (
                workingDir
                    .appendingPathComponent("data", isDirectory: true)
                    .appendingPathComponent("box.log", isDirectory: false),
                "box.log"
            ),
            (
                workingDir.appendingPathComponent("app.log", isDirectory: false),
                "app.log"
            ),
            (
                workingDir.appendingPathComponent("network_extension_error.log", isDirectory: false),
                "network_extension_error.log"
            ),
        ]

        for entry in fixedTargets {
            mirrorIfChanged(
                sourceURL: entry.source,
                destinationName: entry.destinationName,
                logsDir: logsDir,
                fileManager: fileManager
            )
        }

        if let crashSource = newestCoreCrashLog(in: workingDir, fileManager: fileManager) {
            mirrorIfChanged(
                sourceURL: crashSource,
                destinationName: "core_crash.log",
                logsDir: logsDir,
                fileManager: fileManager
            )
        }
    }

    private func nonEmptyFileStamp(for url: URL, fileManager: FileManager) -> FileStamp? {
        guard
            let attrs = try? fileManager.attributesOfItem(atPath: url.path),
            let fileType = attrs[.type] as? FileAttributeType,
            fileType == .typeRegular,
            let size = (attrs[.size] as? NSNumber)?.uint64Value,
            size > 0
        else {
            return nil
        }
        return FileStamp(
            path: url.path,
            size: size,
            modificationDate: attrs[.modificationDate] as? Date
        )
    }

    private func newestCoreCrashLog(in workingDir: URL, fileManager: FileManager) -> URL? {
        var candidates: [(url: URL, stamp: FileStamp)] = []

        let archiveDir = workingDir.appendingPathComponent("crash_reports", isDirectory: true)
        if let subdirs = try? fileManager.contentsOfDirectory(
            at: archiveDir,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) {
            let sortedDirs = subdirs
                .filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
                .sorted { $0.lastPathComponent > $1.lastPathComponent }
            for dir in sortedDirs {
                let goLog = dir.appendingPathComponent("go.log", isDirectory: false)
                if let stamp = nonEmptyFileStamp(for: goLog, fileManager: fileManager) {
                    candidates.append((goLog, stamp))
                    break
                }
            }
        }

        let liveCrash = workingDir.appendingPathComponent("CrashReport-.log", isDirectory: false)
        if let stamp = nonEmptyFileStamp(for: liveCrash, fileManager: fileManager) {
            candidates.append((liveCrash, stamp))
        }

        if candidates.count == 2 {
            let archived = candidates[0]
            let live = candidates[1]
            if let archivedDate = archived.stamp.modificationDate,
               let liveDate = live.stamp.modificationDate {
                return liveDate >= archivedDate ? live.url : archived.url
            }
            return live.url
        }
        return candidates.first?.url
    }

    private func mirrorIfChanged(
        sourceURL: URL,
        destinationName: String,
        logsDir: URL,
        fileManager: FileManager
    ) {
        guard let stamp = nonEmptyFileStamp(for: sourceURL, fileManager: fileManager) else {
            return
        }
        let destinationURL = logsDir.appendingPathComponent(destinationName, isDirectory: false)
        if let previous = lastSynced[destinationName],
           previous == stamp,
           fileManager.fileExists(atPath: destinationURL.path) {
            return
        }

        do {
            try fileManager.createDirectory(at: logsDir, withIntermediateDirectories: true)
            let replacementDir = try fileManager.url(
                for: .itemReplacementDirectory,
                in: .userDomainMask,
                appropriateFor: logsDir,
                create: true
            )
            defer { try? fileManager.removeItem(at: replacementDir) }

            let tempURL = replacementDir.appendingPathComponent(destinationName, isDirectory: false)
            try fileManager.copyItem(at: sourceURL, to: tempURL)

            guard nonEmptyFileStamp(for: tempURL, fileManager: fileManager) != nil else {
                return
            }

            if fileManager.fileExists(atPath: destinationURL.path) {
                _ = try fileManager.replaceItemAt(destinationURL, withItemAt: tempURL)
            } else {
                try fileManager.moveItem(at: tempURL, to: destinationURL)
            }
            lastSynced[destinationName] = stamp
        } catch {
            NSLog("hiddify: failed to mirror \(destinationName): \(error.localizedDescription)")
        }
    }
}


