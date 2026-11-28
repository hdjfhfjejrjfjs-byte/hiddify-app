import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:fpdart/fpdart.dart';
import 'package:go_router/go_router.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/notification/in_app_notification_controller.dart';
import 'package:hiddify/core/preferences/preferences_provider.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/features/profile/notifier/profile_notifier.dart';
import 'package:hiddify/features/profile/overview/profiles_notifier.dart';
import 'package:hiddify/features/profile/widget/profile_tile.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FakeProfilesNotifier extends ProfilesNotifier {
  _FakeProfilesNotifier(this.profiles);
  final List<ProfileEntity> profiles;
  final selectedIds = <String>[];

  @override
  Stream<List<ProfileEntity>> build() => Stream.value(profiles);

  @override
  Future<Unit> selectActiveProfile(String id) async {
    selectedIds.add(id);
    return unit;
  }
}

class _IdleUpdateProfileNotifier extends UpdateProfileNotifier {
  @override
  AsyncValue<Unit?> build(String id) => const AsyncData(null);
}

class _FakeNotifications extends InAppNotificationController {
  @override
  void showErrorToast(String message) {}
}

ProfileEntity _localProfile(String id, String name, {bool active = false}) =>
    ProfileEntity.local(id: id, active: active, name: name, lastUpdate: DateTime(2026));

void main() {
  Future<({ProviderContainer container, GoRouter router, TranslationsEn t})> pumpMainTile(
    WidgetTester tester, {
    required Size surfaceSize,
    required ProfileEntity profile,
    int? proxyCount = 12,
    AppLocale locale = AppLocale.en,
    TextDirection textDirection = TextDirection.ltr,
  }) async {
    SharedPreferences.setMockInitialValues({'haptic_feedback': false});
    tester.view.physicalSize = surfaceSize;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final container = ProviderContainer(
      overrides: [
        translationsProvider.overrideWith((ref) => locale.build()),
        inAppNotificationControllerProvider.overrideWithValue(_FakeNotifications()),
        profileProxyCountProvider(profile.id).overrideWith((ref) => Future.value(proxyCount ?? 0)),
        updateProfileNotifierProvider(profile.id).overrideWith(_IdleUpdateProfileNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.runAsync(() => container.read(sharedPreferencesProvider.future));
    final t = await tester.runAsync(() => container.read(translationsProvider.future));

    final router = GoRouter(
      initialLocation: '/home',
      routes: [
        GoRoute(
          name: 'home',
          path: '/home',
          builder: (_, _) => Directionality(
            textDirection: textDirection,
            child: Scaffold(
              body: Padding(
                padding: const EdgeInsets.all(12),
                child: ProfileTile(profile: profile, isMain: true),
              ),
            ),
          ),
          routes: [
            GoRoute(
              name: 'proxies',
              path: 'proxies',
              builder: (_, _) => const Scaffold(body: Text('PROXIES_PAGE')),
            ),
          ],
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
    await tester.pumpAndSettle();

    return (container: container, router: router, t: t!);
  }

  testWidgets('ProfileTile(isMain: true) shows node count + chevron and navigates to proxies on mobile', (
    tester,
  ) async {
    final profile = _localProfile('p1', 'Main Sub', active: true);
    final (:t, container: _, router: _) = await pumpMainTile(
      tester,
      surfaceSize: const Size(390, 844),
      profile: profile,
      proxyCount: 24,
    );

    expect(find.text(t.pages.profiles.nodeCount(n: 24)), findsOneWidget);
    expect(find.byIcon(Icons.chevron_right_rounded), findsOneWidget);
    expect(find.byIcon(Icons.arrow_drop_down), findsNothing);
    expect(find.byIcon(Icons.keyboard_arrow_down), findsNothing);

    await tester.tap(find.text('Main Sub'));
    await tester.pumpAndSettle();

    expect(find.text('PROXIES_PAGE'), findsOneWidget);
  });

  testWidgets('ProfileTile(isMain: true) navigates to proxies on desktop width (>= 841px)', (tester) async {
    final profile = _localProfile('p1', 'Desktop Sub', active: true);
    await pumpMainTile(
      tester,
      surfaceSize: const Size(1280, 800),
      profile: profile,
      proxyCount: 8,
    );

    expect(find.byIcon(Icons.chevron_right_rounded), findsOneWidget);

    await tester.tap(find.text('Desktop Sub'));
    await tester.pumpAndSettle();

    expect(find.text('PROXIES_PAGE'), findsOneWidget);
  });

  testWidgets('ProfileTile(isMain: true) handles 0 proxies and narrow Persian RTL without overflow', (tester) async {
    final profile = _localProfile('p-fa', 'اشتراک بسیار طولانی با نام کامل برای آزمایش عرض کم', active: true);
    await pumpMainTile(
      tester,
      surfaceSize: const Size(320, 640),
      profile: profile,
      proxyCount: 0,
      locale: AppLocale.fa,
      textDirection: TextDirection.rtl,
    );

    expect(find.byIcon(Icons.chevron_right_rounded), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('ProfileTile(isMain: false) in a modal sheet over proxies pops ONLY the sheet (no double pop)', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({'haptic_feedback': false});
    final p1 = _localProfile('p1', 'Active Sub', active: true);
    final p2 = _localProfile('p2', 'Backup Sub');
    final fakeProfiles = _FakeProfilesNotifier([p1, p2]);

    final container = ProviderContainer(
      overrides: [
        translationsProvider.overrideWith((ref) => AppLocale.en.build()),
        inAppNotificationControllerProvider.overrideWithValue(_FakeNotifications()),
        profilesNotifierProvider.overrideWith(() => fakeProfiles),
        updateProfileNotifierProvider(p1.id).overrideWith(_IdleUpdateProfileNotifier.new),
        updateProfileNotifierProvider(p2.id).overrideWith(_IdleUpdateProfileNotifier.new),
      ],
    );
    addTearDown(container.dispose);
    await tester.runAsync(() => container.read(sharedPreferencesProvider.future));
    await tester.runAsync(() => container.read(translationsProvider.future));

    final router = GoRouter(
      initialLocation: '/home/proxies',
      routes: [
        GoRoute(
          name: 'home',
          path: '/home',
          builder: (_, _) => const Scaffold(body: Text('HOME_PAGE')),
          routes: [
            GoRoute(
              name: 'proxies',
              path: 'proxies',
              builder: (context, _) => Scaffold(
                body: Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Text('PROXIES_PAGE_UNDERNEATH'),
                      ElevatedButton(
                        onPressed: () {
                          showModalBottomSheet<void>(
                            context: context,
                            builder: (_) => Padding(
                              padding: const EdgeInsets.all(12),
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  ProfileTile(profile: p1),
                                  ProfileTile(profile: p2),
                                ],
                              ),
                            ),
                          );
                        },
                        child: const Text('OPEN_SHEET'),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ],
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
    await tester.pumpAndSettle();

    expect(find.text('PROXIES_PAGE_UNDERNEATH'), findsOneWidget);

    // Open the profile switcher sheet above the proxies page.
    await tester.tap(find.text('OPEN_SHEET'));
    await tester.pumpAndSettle();
    expect(find.text('Backup Sub'), findsOneWidget);

    // Tap the second profile: selects it and pops ONLY the bottom sheet.
    await tester.tap(find.text('Backup Sub'));
    await tester.pumpAndSettle();

    expect(fakeProfiles.selectedIds, ['p2']);
    expect(find.text('Backup Sub'), findsNothing);
    expect(find.text('PROXIES_PAGE_UNDERNEATH'), findsOneWidget);
    expect(find.text('HOME_PAGE'), findsNothing);
  });
}
