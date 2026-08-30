import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  final fixturePath = Platform.environment['PAGELET_PAGE_SCENE_PATH'];

  test(
    'decodes a Rust-encoded page scene without losing render fields',
    () {
      final batch = const PageSceneDecoder().decode(
        File(fixturePath!).readAsBytesSync(),
      );
      final page = batch.pages.single;
      final paragraph = page.paragraphs.single;
      final paint = page.textPaints.single;

      expect(batch.schemaVersion, 3);
      expect(page.pageIndex, 2);
      expect(page.size.width, 320);
      expect(page.size.height, 480);
      expect(page.textBackendId, 9);
      expect(page.fontFingerprint, 10);
      expect(paragraph.paragraphId, 77);
      expect(paragraph.text, 'Hello 中🙂');
      expect(paint.paragraphId, paragraph.paragraphId);
      expect(paint.paintOrigin.dx, 12);
      expect(paint.paintOrigin.dy, -4);
      expect(paint.clipTop, 8);
      expect(paint.clipHeight, 100);
      expect(paint.sourceRange!.start, 10);
      expect(paint.sourceRange!.end, 30);
      expect(page.fragments.single.kind, SceneFragmentKind.image);
      expect(page.links.single.textRange!.end, 5);
      expect(page.links.single.resolvedDocument, 'OPS/chapter-2.xhtml');
    },
    skip: fixturePath == null
        ? 'Set PAGELET_PAGE_SCENE_PATH to a Rust-encoded PageBatch.'
        : false,
  );
}
