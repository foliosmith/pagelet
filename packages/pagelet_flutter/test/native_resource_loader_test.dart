import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';

void main() {
  final libraryPath = Platform.environment['PAGELET_LIBRARY_PATH'];
  final epubPath = Platform.environment['PAGELET_RESOURCE_EPUB_PATH'];

  test(
    'reads an EPUB resource through the real owned-buffer C ABI',
    () {
      final engine = PageletEngine(libraryPath: libraryPath!);
      try {
        final book = engine.openBook(epubPath!);
        final resource = book.resources.read(0);

        expect(resource.id, 0);
        expect(resource.path, isNotEmpty);
        expect(resource.mediaType, isNotEmpty);
        expect(resource.bytes, isNotEmpty);
      } finally {
        engine.dispose();
      }
    },
    skip: libraryPath == null || epubPath == null
        ? 'Set PAGELET_LIBRARY_PATH and PAGELET_RESOURCE_EPUB_PATH.'
        : false,
  );
}
