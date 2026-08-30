import 'package:flutter/widgets.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

sealed class ReaderTapTarget {
  const ReaderTapTarget();
}

final class ReaderLinkTap extends ReaderTapTarget {
  const ReaderLinkTap(this.link);

  final PageLinkRegion link;
}

final class ReaderImageTap extends ReaderTapTarget {
  const ReaderImageTap(this.fragment);

  final SceneFragment fragment;
}

ReaderTapTarget? resolveReaderTap(PageScene page, Offset position) {
  for (final link in page.links) {
    if (link.rect.contains(position)) {
      return ReaderLinkTap(link);
    }
  }
  for (final fragment in page.fragments) {
    if (fragment.kind == SceneFragmentKind.image &&
        fragment.rect.contains(position)) {
      return ReaderImageTap(fragment);
    }
  }
  return null;
}
