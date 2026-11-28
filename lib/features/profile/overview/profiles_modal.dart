import 'package:flutter/material.dart';
import 'package:gap/gap.dart';
import 'package:hiddify/core/localization/translations.dart';
import 'package:hiddify/core/model/failures.dart';
import 'package:hiddify/core/router/bottom_sheets/bottom_sheets_notifier.dart';
import 'package:hiddify/core/router/dialog/dialog_notifier.dart';
import 'package:hiddify/features/profile/notifier/profiles_update_notifier.dart';
import 'package:hiddify/features/profile/overview/profiles_notifier.dart';
import 'package:hiddify/features/profile/widget/profile_tile.dart';
import 'package:hiddify/utils/utils.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class ProfilesModal extends HookConsumerWidget {
  const ProfilesModal({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = ref.watch(translationsProvider).requireValue;
    final theme = Theme.of(context);
    final asyncProfiles = ref.watch(profilesNotifierProvider);

    ref.listen(profilesNotifierProvider, (_, next) {
      if (next.hasValue && next.value!.isEmpty) {
        if (Navigator.of(context).canPop() && (ModalRoute.of(context)?.isCurrent ?? true)) {
          Navigator.of(context).pop();
        }
      }
    });

    final initialSize = PlatformUtils.isDesktop ? .60 : .50;
    return SafeArea(
      child: asyncProfiles.when(
        data: (data) => DraggableScrollableSheet(
          initialChildSize: initialSize,
          minChildSize: 0.35,
          maxChildSize: 0.85,
          expand: false,
          builder: (context, scrollController) => Column(
            children: [
              Padding(
                padding: const EdgeInsets.only(top: 10, bottom: 4),
                child: Center(
                  child: Container(
                    width: 36,
                    height: 4,
                    decoration: BoxDecoration(
                      color: theme.colorScheme.onSurfaceVariant.withValues(alpha: .4),
                      borderRadius: BorderRadius.circular(2),
                    ),
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsetsDirectional.fromSTEB(16, 0, 8, 4),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        t.pages.profiles.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.titleLarge,
                      ),
                    ),
                    IconButton(
                      tooltip: t.pages.profiles.add,
                      icon: Icon(Icons.add_rounded, color: theme.colorScheme.primary),
                      onPressed: () {
                        final bottomSheets = ref.read(bottomSheetsNotifierProvider.notifier);
                        if (Navigator.of(context).canPop() && (ModalRoute.of(context)?.isCurrent ?? true)) {
                          Navigator.of(context).pop();
                        }
                        bottomSheets.showAddProfile();
                      },
                    ),
                  ],
                ),
              ),
              const Divider(height: 1),
              Expanded(
                child: ScrollConfiguration(
                  behavior: ScrollConfiguration.of(context).copyWith(scrollbars: false),
                  child: Stack(
                    children: [
                      ListView.separated(
                        padding: const EdgeInsets.fromLTRB(12, 12, 12, 88),
                        separatorBuilder: (context, index) => const Gap(12),
                        controller: scrollController,
                        itemBuilder: (context, index) =>
                            ProfileTile(key: ValueKey(data[index].id), profile: data[index]),
                        itemCount: data.length,
                      ),
                      Positioned(
                        bottom: 0,
                        left: 16,
                        right: 16,
                        child: Padding(
                          padding: const EdgeInsets.only(bottom: 12),
                          child: Wrap(
                            spacing: 8,
                            runSpacing: 8,
                            alignment: WrapAlignment.center,
                            crossAxisAlignment: WrapCrossAlignment.center,
                            children: [
                              FilledButton.icon(
                                label: Text(t.common.sort, maxLines: 1, overflow: TextOverflow.ellipsis),
                                icon: const Icon(Icons.sort_rounded),
                                onPressed: () => ref.read(dialogNotifierProvider.notifier).showSortProfiles(),
                              ),
                              FilledButton.icon(
                                label: Text(
                                  t.pages.profiles.updateSubscriptions,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                ),
                                icon: const Icon(Icons.sync_rounded),
                                onPressed: () => ref.read(foregroundProfilesUpdateNotifierProvider.notifier).trigger(),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
        loading: () => const Center(child: CircularProgressIndicator()),
        error: (error, stackTrace) => Padding(
          padding: const EdgeInsets.all(16),
          child: Text(t.presentShortError(error)),
        ),
      ),
    );
  }
}
