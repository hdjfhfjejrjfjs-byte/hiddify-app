import 'package:flutter/material.dart';
import 'package:gap/gap.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/router/bottom_sheets/bottom_sheets_notifier.dart';
import 'package:hiddify/features/profile/model/profile_entity.dart';
import 'package:hiddify/features/proxy/keep_top/keep_top.dart';
import 'package:hiddify/gen/fonts.gen.dart';
import 'package:hiddify/utils/platform_utils.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// Pinned header above the proxies list: the active subscription bar (tapping
/// opens the keep-top filter dialog, with a trailing button to switch profiles).
class ProxiesHeader extends ConsumerWidget {
  const ProxiesHeader({
    super.key,
    required this.activeProfile,
    required this.isConnected,
    required this.isConnecting,
    required this.totalCount,
  });

  final ProfileEntity? activeProfile;
  final bool isConnected;
  final bool isConnecting;
  final int totalCount;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profile = activeProfile;
    if (profile == null) return const SizedBox.shrink();

    final colorScheme = Theme.of(context).colorScheme;

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsetsDirectional.fromSTEB(12, 4, 12, 10),
          child: _SubscriptionFilterBar(
            profile: profile,
            isConnected: isConnected,
            isConnecting: isConnecting,
            totalCount: totalCount,
            onFilterTap: () {
              if (totalCount > 0) {
                ref.read(keepTopNotifierProvider.notifier).choose(profile, totalCount);
              } else {
                ref.read(bottomSheetsNotifierProvider.notifier).showProfilesOverview();
              }
            },
            onSwitchProfileTap: () => ref.read(bottomSheetsNotifierProvider.notifier).showProfilesOverview(),
          ),
        ),
        Divider(height: 1, thickness: 1, color: colorScheme.outlineVariant.withValues(alpha: .4)),
      ],
    );
  }
}

class _SubscriptionFilterBar extends ConsumerWidget {
  const _SubscriptionFilterBar({
    required this.profile,
    required this.isConnected,
    required this.isConnecting,
    required this.totalCount,
    required this.onFilterTap,
    required this.onSwitchProfileTap,
  });

  final ProfileEntity profile;
  final bool isConnected;
  final bool isConnecting;
  final int totalCount;
  final VoidCallback onFilterTap;
  final VoidCallback onSwitchProfileTap;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;
    final t = ref.watch(translationsProvider).requireValue;
    final dark = theme.brightness == Brightness.dark;

    final keepTop = profile.keepTop;
    final isFiltering =
        profile.keepTags.isNotEmpty || keepTop == keepTopAnswering || (keepTop > 0 && totalCount > keepTop);

    final (Widget statusIndicator, String subtitle, Color subtitleColor) = isConnected
        ? (
            _StatusDot(color: dark ? Colors.lightGreen : Colors.green),
            t.pages.profiles.nodeCount(n: totalCount),
            colorScheme.onSurfaceVariant,
          )
        : isConnecting
        ? (
            SizedBox.square(
              dimension: 11,
              child: CircularProgressIndicator(strokeWidth: 1.8, color: colorScheme.primary),
            ),
            t.pages.proxies.connectingToProxy,
            colorScheme.primary,
          )
        : (_StatusDot(color: colorScheme.outline), t.pages.proxies.offlineTapToConnect, colorScheme.onSurfaceVariant);

    return Material(
      color: colorScheme.surfaceContainerLow,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(14),
        side: BorderSide(color: colorScheme.outlineVariant.withValues(alpha: .6)),
      ),
      clipBehavior: Clip.antiAlias,
      child: IntrinsicHeight(
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: Semantics(
                button: true,
                label: '${profile.name}, ${t.dialogs.keepTop.howMany}',
                child: InkWell(
                  onTap: onFilterTap,
                  child: Padding(
                    padding: const EdgeInsetsDirectional.fromSTEB(12, 10, 8, 10),
                    child: Row(
                      children: [
                        Container(
                          width: 36,
                          height: 36,
                          alignment: Alignment.center,
                          decoration: BoxDecoration(
                            color: isFiltering
                                ? colorScheme.primaryContainer
                                : colorScheme.primary.withValues(alpha: .12),
                            borderRadius: BorderRadius.circular(10),
                          ),
                          child: Icon(
                            isFiltering ? Icons.filter_alt_rounded : Icons.filter_alt_outlined,
                            size: 20,
                            color: isFiltering ? colorScheme.onPrimaryContainer : colorScheme.primary,
                          ),
                        ),
                        const Gap(12),
                        Expanded(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                profile.name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: theme.textTheme.titleSmall?.copyWith(
                                  fontWeight: FontWeight.w600,
                                  fontFamily: PlatformUtils.isWindows ? FontFamily.emoji : null,
                                ),
                              ),
                              const Gap(2),
                              Row(
                                children: [
                                  statusIndicator,
                                  const Gap(6),
                                  Flexible(
                                    child: Text(
                                      subtitle,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: theme.textTheme.bodySmall?.copyWith(color: subtitleColor),
                                    ),
                                  ),
                                ],
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
            VerticalDivider(
              width: 1,
              thickness: 1,
              indent: 10,
              endIndent: 10,
              color: colorScheme.outlineVariant.withValues(alpha: .5),
            ),
            Tooltip(
              message: t.pages.profiles.switchProfile,
              child: InkWell(
                onTap: onSwitchProfileTap,
                child: Padding(
                  padding: const EdgeInsetsDirectional.symmetric(horizontal: 12),
                  child: Center(child: Icon(Icons.swap_horiz_rounded, size: 20, color: colorScheme.onSurfaceVariant)),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _StatusDot extends StatelessWidget {
  const _StatusDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) => Container(
    width: 7,
    height: 7,
    decoration: BoxDecoration(color: color, shape: BoxShape.circle),
  );
}
