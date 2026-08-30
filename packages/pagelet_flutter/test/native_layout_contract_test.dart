import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';
import 'package:pagelet_flutter/src/engine.dart' show PageletEngineTesting;

void main() {
  final libraryPath = Platform.environment['PAGELET_LIBRARY_PATH'];
  final epubPath = Platform.environment['PAGELET_CONTRACT_EPUB_PATH'];

  test(
    'cancels stale work, reuses the layout, and releases native buffers',
    () {
      final engine = PageletEngine(libraryPath: libraryPath!);
      final measurements = <TextMeasurementBatch>[];
      try {
        final book = engine.openBook(epubPath!);
        final chapter = book.openChapter(0);
        final layout = chapter.createLayout(
          const PageletLayoutOptions(
            viewportWidth: 320,
            viewportHeight: 480,
          ),
        );
        final cancelled = layout.requestPages(const PageletPageRequest());
        expect(cancelled.state, PageletLayoutState.needMeasurements);
        expect(PageletEngineTesting.debugLiveBufferCount(engine), 0);
        final staleMeasurement = const TextMeasureBridge(
          fontFingerprint: 0,
        ).measureBatch(cancelled.bytes);
        measurements.add(staleMeasurement);

        layout.cancelRequest(cancelled.requestHandle);
        expect(
          () => layout.submitMeasurements(
            cancelled.requestHandle,
            staleMeasurement.wireBytes,
          ),
          throwsA(isA<PageletException>()),
        );

        var current = layout.requestPages(const PageletPageRequest());
        while (current.state == PageletLayoutState.needMeasurements) {
          final measurement = const TextMeasureBridge(
            fontFingerprint: 0,
          ).measureBatch(current.bytes);
          measurements.add(measurement);
          current = layout.submitMeasurements(
            current.requestHandle,
            measurement.wireBytes,
          );
        }
        expect(current.state, PageletLayoutState.pages);
        expect(
            const PageSceneDecoder().decode(current.bytes).pages, isNotEmpty);
        expect(PageletEngineTesting.debugLiveBufferCount(engine), 0);

        book.dispose();
        expect(book.isDisposed, isTrue);
        expect(chapter.isDisposed, isTrue);
        expect(layout.isDisposed, isTrue);
        expect(
          () => layout.requestPages(const PageletPageRequest()),
          throwsStateError,
        );
      } finally {
        for (final measurement in measurements) {
          measurement.dispose();
        }
        engine.dispose();
      }
    },
    skip: libraryPath == null || epubPath == null
        ? 'Set PAGELET_LIBRARY_PATH and PAGELET_CONTRACT_EPUB_PATH.'
        : false,
  );
}
