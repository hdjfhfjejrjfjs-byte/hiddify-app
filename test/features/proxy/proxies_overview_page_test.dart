import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:fpdart/fpdart.dart';
import 'package:go_router/go_router.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/notification/in_app_notification_controller.dart';
import 'package:hiddify/core/preferences/preferences_provider.dart';
import 'package:hiddify/core/router/bottom_sheets/bottom_sheets_notifier.dart';
import 'package:hiddify/core/router/dialog/widgets/proxy_info_dialog.dart';
import 'package:hiddify/features/connection/model/connection_status.dart';
import 'package:hiddify/features/connection/notifier/connection_notifier.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/features/profile/notifier/active_profile_notifier.dart';
import 'package:hiddify/features/proxy/active/active_proxy_notifier.dart';
import 'package:hiddify/features/proxy/active/ip_widget.dart';
import 'package:hiddify/features/proxy/data/proxy_data_providers.dart';
import 'package:hiddify/features/proxy/data/proxy_repository.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top_widgets.dart';
import 'package:hiddify/features/proxy/model/proxy_failure.dart';
import 'package:hiddify/features/proxy/overview/offline_proxies_provider.dart';
import 'package:hiddify/features/proxy/overview/proxies_overview_notifier.dart';
import 'package:hiddify/features/proxy/overview/proxies_overview_page.dart';
import 'package:hiddify/features/proxy/widget/proxy_tile.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FakeProxyRepository implements ProxyRepository {
  final updates = StreamController<Either<ProxyFailure, OutboundGroup?>>.broadcast();
  final selectedCalls = <(String, String)>[];

  @override
  Stream<Either<ProxyFailure, OutboundGroup?>> watchProxies() => updates.stream;

  @override
  TaskEither<ProxyFailure, Unit> selectProxy(String groupTag, String outboundTag) {
    selectedCalls.add((groupTag, outboundTag));
    return TaskEither.right(unit);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}

class _FakeActiveProfile extends ActiveProfile {
  _FakeActiveProfile(this.profile);
  final ProfileEntity? profile;

  @override
  Stream<ProfileEntity?> build() => Stream.value(profile);
}

class _FakeConnectionNotifier extends ConnectionNotifier {
  _FakeConnectionNotifier(this.controller, this.initial);

  final StreamController<ConnectionStatus> controller;
  final ConnectionStatus initial;
  int mayConnectCalls = 0;

  @override
  Stream<ConnectionStatus> build() async* {
    yield initial;
    yield* controller.stream;
  }

  @override
  Future<void> mayConnect() async {
    mayConnectCalls++;
    if (state case AsyncData(value: Disconnected())) {
      state = const AsyncData(Connecting());
      controller.add(const Connecting());
    }
  }
}

class _FakeBottomSheetsNotifier extends BottomSheetsNotifier {
  int showProfilesOverviewCalls = 0;

  @override
  void build() {}

  @override
  Future<void> showProfilesOverview() async {
    showProfilesOverviewCalls++;
  }
}

class _FakeKeepTopNotifier extends KeepTopNotifier {
  final chooseCalls = <(String, int)>[];

  @override
  void build() {}

  @override
  Future<void> choose(ProfileEntity profile, int count) async {
    chooseCalls.add((profile.id, count));
  }
}

class _FakeNotifications extends InAppNotificationController {
  @override
  void showErrorToast(String message) {}
}

class _EmptyMainOutbounds extends MainOutboundsNotifier {
  @override
  Stream<OutboundGroupList> build() => const Stream.empty();
}

final _activeProfile = ProfileEntity.local(
  id: 'sub-1',
  active: true,
  name: 'My Subscription',
  lastUpdate: DateTime(2026),
);

OutboundInfo _liveNode({
  required String tag,
  String? tagDisplay,
  String type = 'VLESS',
  int delay = 120,
  bool resting = false,
  String countryCode = '',
}) => OutboundInfo(
  tag: tag,
  tagDisplay: tagDisplay ?? tag,
  type: type,
  urlTestDelay: resting ? 0 : delay,
  sortDelay: delay,
  resting: resting,
  ipinfo: countryCode.isEmpty ? null : IpInfo(countryCode: countryCode, ip: '1.1.1.1'),
);

void main() {
  late _FakeProxyRepository repository;
  late StreamController<ConnectionStatus> connectionController;
  late _FakeConnectionNotifier fakeConnection;
  late _FakeBottomSheetsNotifier fakeBottomSheets;
  late _FakeKeepTopNotifier fakeKeepTop;
  late ProviderContainer container;
  late GoRouter router;

  Future<Translations> pumpPage(
    WidgetTester tester, {
    ConnectionStatus initialConnection = const Disconnected(),
    List<ProfileProxy> offlineProxies = const [
      (tag: '🇯🇵 Tokyo 01 § 0', type: 'vless'),
      (tag: '🇩🇪 Frankfurt 02 § 1', type: 'trojan'),
      (tag: 'Home Server', type: 'vmess'),
    ],
    OutboundGroup? liveGroup,
    bool showSleepingProxies = true,
  }) async {
    SharedPreferences.setMockInitialValues({'haptic_feedback': false, 'show_sleeping_proxies': showSleepingProxies});
    repository = _FakeProxyRepository();
    connectionController = StreamController<ConnectionStatus>.broadcast();
    fakeConnection = _FakeConnectionNotifier(connectionController, initialConnection);
    fakeBottomSheets = _FakeBottomSheetsNotifier();
    fakeKeepTop = _FakeKeepTopNotifier();

    container = ProviderContainer(
      overrides: [
        proxyRepositoryProvider.overrideWithValue(repository),
        inAppNotificationControllerProvider.overrideWithValue(_FakeNotifications()),
        translationsProvider.overrideWith((ref) => AppLocale.en.build()),
        activeProfileProvider.overrideWith(() => _FakeActiveProfile(_activeProfile)),
        activeProfileProxiesProvider.overrideWith((ref) => Future.value(offlineProxies)),
        profileProxyCountProvider(_activeProfile.id).overrideWith((ref) => Future.value(offlineProxies.length)),
        connectionNotifierProvider.overrideWith(() => fakeConnection),
        bottomSheetsNotifierProvider.overrideWith(() => fakeBottomSheets),
        keepTopNotifierProvider.overrideWith(() => fakeKeepTop),
      ],
    );
    addTearDown(() async {
      await repository.updates.close();
      await connectionController.close();
      container.dispose();
    });

    await tester.runAsync(() => container.read(sharedPreferencesProvider.future));
    final t = await tester.runAsync(() => container.read(translationsProvider.future));
    // keep the notifier alive until container.dispose so disposeDelay does not leave a pending fake timer
    container.listen(proxiesOverviewNotifierProvider, (_, _) {});

    router = GoRouter(
      initialLocation: '/home/proxies',
      routes: [
        GoRoute(
          name: 'home',
          path: '/home',
          builder: (_, _) => const Scaffold(body: Text('HOME_PAGE')),
          routes: [GoRoute(name: 'proxies', path: 'proxies', builder: (_, _) => const ProxiesOverviewPage())],
        ),
      ],
    );
    addTearDown(router.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp.router(routerConfig: router),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    if (liveGroup != null) {
      repository.updates.add(right(liveGroup));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
    }

    return t!;
  }

  testWidgets('disconnected state renders offline nodes, country flags, offline status, and hides FABs', (
    tester,
  ) async {
    final t = await pumpPage(tester);

    expect(find.text('🇯🇵 Tokyo 01'), findsOneWidget);
    expect(find.text('🇩🇪 Frankfurt 02'), findsOneWidget);
    expect(find.text('Home Server'), findsOneWidget);
    expect(find.text(t.pages.proxies.connectToSeeProxies), findsNothing);
    expect(find.text(t.pages.proxies.offlineTapToConnect), findsOneWidget);
    expect(find.byType(FloatingActionButton), findsNothing);
    // Tokyo & Frankfurt carry tag-derived country codes and render IPCountryFlag cleanly.
    expect(find.byType(IPCountryFlag), findsNWidgets(2));
  });

  testWidgets('tapping an offline node triggers connectAndSelectProxy and highlights that row as connecting', (
    tester,
  ) async {
    final t = await pumpPage(tester);

    await tester.tap(find.text('🇩🇪 Frankfurt 02'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    expect(fakeConnection.mayConnectCalls, 1);
    expect(container.read(pendingProxySelectionProvider), (profileId: 'sub-1', outboundTag: '🇩🇪 Frankfurt 02 § 1'));
    expect(find.text(t.pages.proxies.connectingToProxy), findsOneWidget);

    final tappedTile = tester.widget<ProxyTile>(find.byKey(const ValueKey('🇩🇪 Frankfurt 02 § 1')));
    expect(tappedTile.selected, isTrue);
    expect(tappedTile.testing, isTrue);

    final otherTile = tester.widget<ProxyTile>(find.byKey(const ValueKey('🇯🇵 Tokyo 01 § 0')));
    expect(otherTile.selected, isFalse);
    expect(otherTile.testing, isFalse);
  });

  testWidgets('AppBar removes KeepTopAction and page has no inline search TextField', (tester) async {
    await pumpPage(
      tester,
      initialConnection: const Connected(),
      offlineProxies: [for (var i = 0; i < 30; i++) (tag: 'node-$i', type: 'vless')],
      liveGroup: OutboundGroup(
        tag: 'select',
        selected: 'jp-1',
        items: [_liveNode(tag: 'jp-1', tagDisplay: 'Tokyo Alpha', countryCode: 'JP')],
      ),
    );

    expect(find.byType(KeepTopAction), findsNothing);
    expect(find.byType(TextField), findsNothing);
  });

  testWidgets('tapping top subscription bar opens KeepTop filter dialog; trailing icon switches profile', (
    tester,
  ) async {
    await pumpPage(tester);

    expect(find.text('My Subscription'), findsOneWidget);
    await tester.tap(find.text('My Subscription'));
    await tester.pump();

    expect(fakeKeepTop.chooseCalls, [('sub-1', 3)]);
    expect(fakeBottomSheets.showProfilesOverviewCalls, 0);

    await tester.tap(find.byIcon(Icons.swap_horiz_rounded));
    await tester.pump();

    expect(fakeBottomSheets.showProfilesOverviewCalls, 1);
  });

  testWidgets('Escape pops back to home', (tester) async {
    await pumpPage(tester);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.text('HOME_PAGE'), findsOneWidget);
    expect(find.byType(ProxiesOverviewPage), findsNothing);
  });

  testWidgets('connected state switches proxy on tap and stays on ProxiesOverviewPage', (tester) async {
    await pumpPage(
      tester,
      initialConnection: const Connected(),
      liveGroup: OutboundGroup(
        tag: 'select',
        selected: 'jp-1',
        items: [
          _liveNode(tag: 'jp-1', tagDisplay: 'Tokyo Alpha', countryCode: 'JP'),
          _liveNode(tag: 'us-1', tagDisplay: 'US Beta', countryCode: 'US'),
        ],
      ),
    );

    expect(find.byType(FloatingActionButton), findsNWidgets(2));

    await tester.tap(find.text('US Beta'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    expect(repository.selectedCalls, [('select', 'us-1')]);
    expect(find.byType(ProxiesOverviewPage), findsOneWidget);
    expect(find.text('HOME_PAGE'), findsNothing);
  });

  testWidgets('ProxyInfoDialog renders offline proxy cleanly without offering live tests', (tester) async {
    SharedPreferences.setMockInitialValues({'haptic_feedback': false});
    final offline = offlineProxyToOutboundInfo((tag: '🇯🇵 Tokyo 01 § 0', type: 'vless'));
    final dialogConnController = StreamController<ConnectionStatus>.broadcast();
    final dialogContainer = ProviderContainer(
      overrides: [
        translationsProvider.overrideWith((ref) => AppLocale.en.build()),
        activeProfileProvider.overrideWith(() => _FakeActiveProfile(_activeProfile)),
        connectionNotifierProvider.overrideWith(
          () => _FakeConnectionNotifier(dialogConnController, const Disconnected()),
        ),
        mainOutboundsNotifierProvider.overrideWith(_EmptyMainOutbounds.new),
      ],
    );
    addTearDown(() async {
      await dialogConnController.close();
      dialogContainer.dispose();
    });
    await tester.runAsync(() => dialogContainer.read(sharedPreferencesProvider.future));
    final t = await tester.runAsync(() => dialogContainer.read(translationsProvider.future));
    dialogContainer.listen(proxiesOverviewNotifierProvider, (_, _) {});

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: dialogContainer,
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(child: ProxyInfoDialog(outboundInfo: offline)),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    expect(find.text('🇯🇵 Tokyo 01'), findsOneWidget);
    expect(find.text('JP'), findsOneWidget);
    expect(find.text(t!.dialogs.proxyInfo.tapToTest), findsNothing);
    expect(tester.takeException(), isNull);
  });
}
