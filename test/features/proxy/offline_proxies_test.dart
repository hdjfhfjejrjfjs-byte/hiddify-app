import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:fpdart/fpdart.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/notification/in_app_notification_controller.dart';
import 'package:hiddify/core/preferences/preferences_provider.dart';
import 'package:hiddify/features/connection/model/connection_status.dart';
import 'package:hiddify/features/connection/notifier/connection_notifier.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/features/profile/notifier/active_profile_notifier.dart';
import 'package:hiddify/features/proxy/data/proxy_data_providers.dart';
import 'package:hiddify/features/proxy/data/proxy_repository.dart';
import 'package:hiddify/features/proxy/model/proxy_failure.dart';
import 'package:hiddify/features/proxy/overview/offline_proxies_provider.dart';
import 'package:hiddify/features/proxy/overview/proxies_overview_notifier.dart';
import 'package:hiddify/hiddifycore/generated/v2/hcore/hcore.pb.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FakeProxyRepository implements ProxyRepository {
  final updates = StreamController<Either<ProxyFailure, OutboundGroup?>>.broadcast();
  final selectedCalls = <(String groupTag, String outboundTag)>[];

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
  _FakeActiveProfile(this.controller, this.initial);

  final StreamController<ProfileEntity?> controller;
  final ProfileEntity? initial;

  @override
  Stream<ProfileEntity?> build() async* {
    yield initial;
    yield* controller.stream;
  }
}

class _FakeConnectionNotifier extends ConnectionNotifier {
  _FakeConnectionNotifier(this.controller, this.initial);

  final StreamController<ConnectionStatus> controller;
  final ConnectionStatus initial;
  int mayConnectCalls = 0;
  int disconnectCalls = 0;

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

  @override
  Future<void> toggleConnection() async {
    disconnectCalls++;
  }
}

class _FakeNotifications extends InAppNotificationController {
  final errors = <String>[];

  @override
  void showErrorToast(String message) => errors.add(message);
}

ProfileEntity _profile(String id) =>
    ProfileEntity.local(id: id, active: true, name: 'Profile $id', lastUpdate: DateTime(2026));

OutboundInfo _outbound({
  required String tag,
  String? tagDisplay,
  String type = 'VLESS',
  String countryCode = '',
  String org = '',
  String ip = '',
}) => OutboundInfo(
  tag: tag,
  tagDisplay: tagDisplay ?? tag,
  type: type,
  ipinfo: IpInfo(countryCode: countryCode, org: org, ip: ip),
);

void main() {
  group('offlineProxyToOutboundInfo', () {
    test('populates countryCode from tag without fabricating an IP address', () {
      final info = offlineProxyToOutboundInfo((tag: '🇯🇵 Tokyo 01 § 0', type: 'vless'));
      expect(info.tag, '🇯🇵 Tokyo 01 § 0');
      expect(info.tagDisplay, '🇯🇵 Tokyo 01');
      expect(info.type, 'VLESS');
      expect(info.ipinfo.countryCode, 'JP');
      expect(info.ipinfo.ip, isEmpty);
    });

    test('leaves ipinfo countryCode empty when tag has no country', () {
      final info = offlineProxyToOutboundInfo((tag: 'Home Server', type: 'trojan'));
      expect(info.tagDisplay, 'Home Server');
      expect(info.type, 'Trojan');
      expect(info.ipinfo.countryCode, isEmpty);
      expect(info.ipinfo.ip, isEmpty);
    });
  });

  group('matchesProxyQuery', () {
    final tokyo = _outbound(
      tag: '🇯🇵 Tokyo Fast § 0',
      tagDisplay: '🇯🇵 Tokyo Fast',
      countryCode: 'JP',
      org: 'Cloudflare',
    );
    final frankfurt = _outbound(
      tag: 'DE-Frankfurt-01',
      tagDisplay: 'DE-Frankfurt-01',
      type: 'Trojan',
      countryCode: 'DE',
      org: 'Hetzner Online',
    );
    final usTagOnly = _outbound(tag: '🇺🇸 Los Angeles', tagDisplay: '🇺🇸 Los Angeles', type: 'VMess');

    test('matches all proxies for empty or whitespace query', () {
      expect(matchesProxyQuery(tokyo, ''), isTrue);
      expect(matchesProxyQuery(tokyo, '   '), isTrue);
    });

    test('matches name case-insensitively', () {
      expect(matchesProxyQuery(tokyo, 'tokyo'), isTrue);
      expect(matchesProxyQuery(tokyo, 'TOKYO FAST'), isTrue);
    });

    test('supports multi-word AND matching across fields', () {
      expect(matchesProxyQuery(tokyo, 'japan vless cloudflare'), isTrue);
      expect(matchesProxyQuery(frankfurt, 'de trojan hetzner'), isTrue);
      expect(matchesProxyQuery(tokyo, 'japan trojan'), isFalse);
    });

    test('matches protocol type', () {
      expect(matchesProxyQuery(tokyo, 'vless'), isTrue);
      expect(matchesProxyQuery(frankfurt, 'trojan'), isTrue);
      expect(matchesProxyQuery(usTagOnly, 'vmess'), isTrue);
    });

    test('matches country code and English country name', () {
      expect(matchesProxyQuery(tokyo, 'JP'), isTrue);
      expect(matchesProxyQuery(tokyo, 'Japan'), isTrue);
      expect(matchesProxyQuery(frankfurt, 'Germany'), isTrue);
      expect(matchesProxyQuery(usTagOnly, 'US'), isTrue);
      expect(matchesProxyQuery(usTagOnly, 'United States'), isTrue);
    });

    test('matches IP organization', () {
      expect(matchesProxyQuery(tokyo, 'cloudflare'), isTrue);
      expect(matchesProxyQuery(frankfurt, 'hetzner'), isTrue);
    });

    test('returns false when query does not match any field', () {
      expect(matchesProxyQuery(tokyo, 'singapore'), isFalse);
      expect(matchesProxyQuery(frankfurt, 'wireguard'), isFalse);
    });
  });

  group('sortOfflineProxies', () {
    final items = [
      _outbound(tag: 'c', tagDisplay: 'Charlie'),
      _outbound(tag: 'a', tagDisplay: 'Alpha'),
      _outbound(tag: 'b', tagDisplay: 'bravo'),
    ];

    test('sorts alphabetically for ProxiesSort.name without mutating input', () {
      final originalOrder = [for (final item in items) item.tag];
      final sorted = sortOfflineProxies(items, ProxiesSort.name);
      expect([for (final item in sorted) item.tag], ['a', 'b', 'c']);
      expect([for (final item in items) item.tag], originalOrder);
    });

    test('preserves original order for delay and unsorted', () {
      expect([for (final item in sortOfflineProxies(items, ProxiesSort.delay)) item.tag], ['c', 'a', 'b']);
      expect([for (final item in sortOfflineProxies(items, ProxiesSort.unsorted)) item.tag], ['c', 'a', 'b']);
    });
  });

  group('PendingProxySelection & connectAndSelectProxy', () {
    late _FakeProxyRepository repository;
    late StreamController<ProfileEntity?> profileController;
    late StreamController<ConnectionStatus> connectionController;
    late _FakeConnectionNotifier fakeConnection;
    late ProviderContainer container;

    Future<void> setUpContainer({
      ConnectionStatus initialConnection = const Disconnected(),
      ProfileEntity? initialProfile,
    }) async {
      SharedPreferences.setMockInitialValues({'haptic_feedback': false});
      repository = _FakeProxyRepository();
      profileController = StreamController<ProfileEntity?>.broadcast();
      connectionController = StreamController<ConnectionStatus>.broadcast();
      fakeConnection = _FakeConnectionNotifier(connectionController, initialConnection);
      container = ProviderContainer(
        overrides: [
          proxyRepositoryProvider.overrideWithValue(repository),
          inAppNotificationControllerProvider.overrideWithValue(_FakeNotifications()),
          translationsProvider.overrideWith((ref) => AppLocale.en.build()),
          activeProfileProvider.overrideWith(() => _FakeActiveProfile(profileController, initialProfile ?? _profile('p1'))),
          connectionNotifierProvider.overrideWith(() => fakeConnection),
        ],
      );
      addTearDown(() async {
        await repository.updates.close();
        await profileController.close();
        await connectionController.close();
        container.dispose();
      });
      await container.read(sharedPreferencesProvider.future);
      await container.read(translationsProvider.future);
      container.listen(activeProfileProvider, (_, _) {});
      container.listen(connectionNotifierProvider, (_, _) {});
      container.listen(proxiesOverviewNotifierProvider, (_, _) {});
      container.listen(pendingProxySelectionProvider, (_, _) {});
      await Future<void>.delayed(Duration.zero);
    }

    test('disconnected tap records pending selection, connects, then selects when group emits', () async {
      await setUpContainer();

      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-b');
      expect(fakeConnection.mayConnectCalls, 1);
      expect(container.read(pendingProxySelectionProvider), (profileId: 'p1', outboundTag: 'node-b'));

      // Connection comes up -> serviceRunningProvider becomes true -> ProxiesOverviewNotifier subscribes.
      connectionController.add(const Connected());
      await Future<void>.delayed(Duration.zero);
      expect(container.read(serviceRunningProvider), isTrue);

      repository.updates.add(
        right(
          OutboundGroup(
            tag: 'select',
            selected: 'node-a',
            items: [_outbound(tag: 'node-a'), _outbound(tag: 'node-b')],
          ),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);

      expect(repository.selectedCalls, [('select', 'node-b')]);
      expect(container.read(pendingProxySelectionProvider), isNull);
    });

    test('tapping another node while connecting updates pending target without disconnecting', () async {
      await setUpContainer();

      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-b');
      expect(container.read(pendingProxySelectionProvider), (profileId: 'p1', outboundTag: 'node-b'));
      expect(container.read(connectionNotifierProvider).valueOrNull, isA<Connecting>());

      // Tap a different node while still connecting.
      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-c');
      expect(container.read(pendingProxySelectionProvider), (profileId: 'p1', outboundTag: 'node-c'));
      expect(fakeConnection.disconnectCalls, 0);
      expect(container.read(connectionNotifierProvider).valueOrNull, isA<Connecting>());

      connectionController.add(const Connected());
      await Future<void>.delayed(Duration.zero);

      repository.updates.add(
        right(
          OutboundGroup(
            tag: 'select',
            selected: 'node-a',
            items: [_outbound(tag: 'node-a'), _outbound(tag: 'node-b'), _outbound(tag: 'node-c')],
          ),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);

      expect(repository.selectedCalls, [('select', 'node-c')]);
      expect(container.read(pendingProxySelectionProvider), isNull);
    });

    test('already connected switches directly without setting pending state', () async {
      await setUpContainer(initialConnection: const Connected());
      await Future<void>.delayed(Duration.zero);

      repository.updates.add(
        right(
          OutboundGroup(
            tag: 'select',
            selected: 'node-a',
            items: [_outbound(tag: 'node-a'), _outbound(tag: 'node-b')],
          ),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(container.read(proxiesOverviewNotifierProvider).valueOrNull, isNotNull);

      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-b');
      expect(fakeConnection.mayConnectCalls, 0);
      expect(container.read(pendingProxySelectionProvider), isNull);
      expect(repository.selectedCalls, [('select', 'node-b')]);
    });

    test('clears pending selection when connection transitions back to Disconnected', () async {
      await setUpContainer();

      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-b');
      expect(container.read(pendingProxySelectionProvider), (profileId: 'p1', outboundTag: 'node-b'));

      connectionController.add(const Disconnected());
      await Future<void>.delayed(Duration.zero);

      expect(container.read(pendingProxySelectionProvider), isNull);
    });

    test('clears pending selection when active profile changes', () async {
      await setUpContainer();

      await container.read(proxiesOverviewNotifierProvider.notifier).connectAndSelectProxy('node-b');
      expect(container.read(pendingProxySelectionProvider), (profileId: 'p1', outboundTag: 'node-b'));

      profileController.add(_profile('p2'));
      await Future<void>.delayed(Duration.zero);

      expect(container.read(pendingProxySelectionProvider), isNull);
    });
  });
}
