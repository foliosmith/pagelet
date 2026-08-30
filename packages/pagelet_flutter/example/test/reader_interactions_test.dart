import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

import '../lib/reader_interactions.dart';

void main() {
  test('resolves links before image regions and ignores empty space', () {
    final link = PageLinkRegion(
      rect: const Rect.fromLTWH(0, 0, 20, 20),
      nodeId: 1,
      textRange: null,
      href: '#footnote',
      resolvedDocument: null,
      fragment: 'footnote',
      kind: PageLinkKind.footnote,
    );
    final image = SceneFragment(
      id: 2,
      kind: SceneFragmentKind.image,
      nodeId: 2,
      rect: const Rect.fromLTWH(10, 10, 30, 30),
      text: 'Cover',
      sourceRange: null,
      anchorRange: null,
      lineIndex: null,
      overflow: false,
    );
    final page =
        _page(links: <PageLinkRegion>[link], fragments: <SceneFragment>[image]);

    expect(resolveReaderTap(page, const Offset(15, 15)), isA<ReaderLinkTap>());
    expect(resolveReaderTap(page, const Offset(30, 30)), isA<ReaderImageTap>());
    expect(resolveReaderTap(page, const Offset(50, 50)), isNull);
  });
}

PageScene _page({
  required List<PageLinkRegion> links,
  required List<SceneFragment> fragments,
}) {
  return PageScene(
    pageIndex: 0,
    size: const Size(100, 100),
    startAnchor: null,
    endAnchor: null,
    textBackendId: 0,
    fontFingerprint: 0,
    paragraphs: const <SceneParagraph>[],
    textPaints: const <TextPaintFragment>[],
    fragments: fragments,
    links: links,
    anchors: const <PageAnchorRegion>[],
    selections: const <PageSelectionMap>[],
    semantics: const <PageSemanticNode>[],
    fingerprint: '',
    nextBreakToken: null,
    diagnostics: const <PageDiagnostic>[],
  );
}
