import 'dart:convert';
import 'dart:typed_data';
import 'package:flutter_test/flutter_test.dart';
import 'package:pagelet_flutter/pagelet_flutter.dart';
import 'package:pagelet_flutter/src/engine.dart' show PageletEngineTesting;
import 'package:pagelet_flutter/src/native_api.dart';

void main() {
  group('PageletEngine lifecycle', () {
    test('creates an engine and opens a book path', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);

      final book = engine.openBook('/books/example.epub');

      expect(native.createdEngineHandles, <int>[1]);
      expect(native.openedPaths, <(int, String)>[(1, '/books/example.epub')]);
      expect(book.isDisposed, isFalse);
      expect(engine.isDisposed, isFalse);
    });

    test('opens a borrowed file descriptor', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);

      engine.openBookFileDescriptor(42);

      expect(native.openedFileDescriptors, <(int, int)>[(1, 42)]);
    });

    test('book disposal is idempotent', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);
      final book = engine.openBook('/books/example.epub');

      book.dispose();
      book.dispose();

      expect(book.isDisposed, isTrue);
      expect(native.disposedHandles, <int>[2]);
    });

    test('engine disposal invalidates books with one native call', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);
      final first = engine.openBook('/books/first.epub');
      final second = engine.openBook('/books/second.epub');

      engine.dispose();
      engine.dispose();
      first.dispose();
      second.dispose();

      expect(engine.isDisposed, isTrue);
      expect(first.isDisposed, isTrue);
      expect(second.isDisposed, isTrue);
      expect(native.disposedHandles, <int>[1]);
    });

    test('rejects operations after engine disposal', () {
      final engine = PageletEngineTesting.create(_FakeNativeApi())..dispose();

      expect(() => engine.openBook('/books/example.epub'), throwsStateError);
      expect(() => engine.openBookFileDescriptor(42), throwsStateError);
    });

    test('validates host arguments before native calls', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);

      expect(() => engine.openBook(''), throwsArgumentError);
      expect(() => engine.openBookFileDescriptor(-1), throwsArgumentError);
      expect(native.openedPaths, isEmpty);
      expect(native.openedFileDescriptors, isEmpty);
    });

    test('preserves native status and internal error identifiers', () {
      final native = _FakeNativeApi(
        openPathResult: const PageletNativeHandleResult(
          status: PageletStatus.internal,
          statusCode: 11,
          internalErrorId: 987,
          handle: 0,
        ),
      );
      final engine = PageletEngineTesting.create(native);

      expect(
        () => engine.openBook('/books/example.epub'),
        throwsA(
          isA<PageletException>()
              .having((error) => error.operation, 'operation', 'book_open_path')
              .having((error) => error.status, 'status', PageletStatus.internal)
              .having((error) => error.internalErrorId, 'error id', 987),
        ),
      );
    });

    test('failed disposal leaves the wrapper live for retry', () {
      final native = _FakeNativeApi(
        disposeResults: <PageletNativeStatusResult>[
          const PageletNativeStatusResult(
            status: PageletStatus.internal,
            statusCode: 11,
            internalErrorId: 123,
          ),
          _FakeNativeApi.okStatus,
        ],
      );
      final engine = PageletEngineTesting.create(native);

      expect(engine.dispose, throwsA(isA<PageletException>()));
      expect(engine.isDisposed, isFalse);

      engine.dispose();

      expect(engine.isDisposed, isTrue);
      expect(native.disposedHandles, <int>[1, 1]);
    });

    test('reads one resource only on demand into defensive host bytes', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);
      final book = engine.openBook('/books/example.epub');

      expect(native.resourceReads, isEmpty);
      final resource = book.resources.read(7);

      expect(native.resourceReads, <(int, int)>[(2, 7)]);
      expect(resource.id, 7);
      expect(resource.path, 'EPUB/images/cover.png');
      expect(resource.mediaType, 'image/png');
      expect(resource.bytes, <int>[1, 2, 3]);
      final callerBytes = resource.bytes..[0] = 9;
      expect(callerBytes, <int>[9, 2, 3]);
      expect(resource.bytes, <int>[1, 2, 3]);

      book.dispose();
      expect(() => book.resources.read(7), throwsStateError);
    });

    test('reads and caches typed book metadata and navigation', () {
      final native = _FakeNativeApi();
      final engine = PageletEngineTesting.create(native);
      final book = engine.openBook('/books/example.epub');

      final summary = book.summary;

      expect(summary.title, 'Fixture Book');
      expect(summary.language, 'en');
      expect(summary.spine.single.idref, 'chapter');
      expect(summary.navigation.source, 'epub3-nav');
      expect(summary.navigation.toc.single.label, 'Chapter 1');
      expect(summary.navigation.toc.single.children.single.href, 'note.xhtml');
      expect(summary.diagnostics, isEmpty);
      expect(identical(book.summary, summary), isTrue);
      expect(native.summaryReads, <int>[2]);
    });

    test('preserves native book summary failures', () {
      final native = _FakeNativeApi(
        summaryResult: PageletNativeBytesResult(
          status: PageletStatus.internal,
          statusCode: 11,
          internalErrorId: 44,
          bytes: Uint8List(0),
        ),
      );
      final book = PageletEngineTesting.create(
        native,
      ).openBook('/books/example.epub');

      expect(
        () => book.summary,
        throwsA(
          isA<PageletException>()
              .having((error) => error.operation, 'operation', 'book_summary')
              .having((error) => error.internalErrorId, 'error id', 44),
        ),
      );
    });

    test('rejects a native resource id mismatch', () {
      final native = _FakeNativeApi(resourceIdOffset: 1);
      final engine = PageletEngineTesting.create(native);
      final book = engine.openBook('/books/example.epub');

      expect(() => book.resources.read(7), throwsFormatException);
    });

    test('maps page coordinates to the native UTF-8 hit-test result', () {
      final native = _FakeNativeApi(
        hitTestResult: const PageletNativeHitTestResult(
          status: PageletStatus.ok,
          statusCode: 0,
          internalErrorId: 0,
          found: true,
          affinity: 1,
          nodeId: 7,
          utf8ByteOffset: 9,
          fragmentId: 88,
        ),
      );
      final engine = PageletEngineTesting.create(native);
      final layout =
          engine.openBook('/books/example.epub').openChapter(0).createLayout(
                const PageletLayoutOptions(
                  viewportWidth: 320,
                  viewportHeight: 480,
                ),
              );

      final hit = layout.hitTest(2, const Offset(1.5, 2));

      expect(native.hitTests, <(int, int, int, int)>[(4, 2, 96, 128)]);
      expect(hit!.nodeId, 7);
      expect(hit.utf8ByteOffset, 9);
      expect(hit.fragmentId, 88);
      expect(hit.affinity, PageTextAffinity.downstream);
    });
  });
}

final class _FakeNativeApi implements PageletNativeApi {
  _FakeNativeApi({
    this.openPathResult,
    this.summaryResult,
    this.resourceIdOffset = 0,
    this.hitTestResult,
    List<PageletNativeStatusResult>? disposeResults,
  }) : _disposeResults = disposeResults ?? <PageletNativeStatusResult>[];

  static const okStatus = PageletNativeStatusResult(
    status: PageletStatus.ok,
    statusCode: 0,
    internalErrorId: 0,
  );

  int _nextHandle = 1;
  final PageletNativeHandleResult? openPathResult;
  final PageletNativeBytesResult? summaryResult;
  final int resourceIdOffset;
  final PageletNativeHitTestResult? hitTestResult;
  final List<PageletNativeStatusResult> _disposeResults;
  final List<int> createdEngineHandles = <int>[];
  final List<(int, String)> openedPaths = <(int, String)>[];
  final List<(int, int)> openedFileDescriptors = <(int, int)>[];
  final List<(int, int)> resourceReads = <(int, int)>[];
  final List<int> summaryReads = <int>[];
  final List<(int, int, int, int)> hitTests = <(int, int, int, int)>[];
  final List<int> disposedHandles = <int>[];

  @override
  PageletNativeHandleResult engineCreate() {
    final handle = _nextHandle++;
    createdEngineHandles.add(handle);
    return _okHandle(handle);
  }

  @override
  PageletNativeHandleResult bookOpenPath(int engine, String path) {
    openedPaths.add((engine, path));
    return openPathResult ?? _okHandle(_nextHandle++);
  }

  @override
  PageletNativeHandleResult bookOpenFileDescriptor(int engine, int fd) {
    openedFileDescriptors.add((engine, fd));
    return _okHandle(_nextHandle++);
  }

  @override
  PageletNativeBytesResult bookSummary(int book) {
    summaryReads.add(book);
    return summaryResult ??
        PageletNativeBytesResult(
          status: PageletStatus.ok,
          statusCode: 0,
          internalErrorId: 0,
          bytes: Uint8List.fromList(utf8.encode(_summaryJson)),
        );
  }

  @override
  PageletNativeHandleResult chapterOpen(int book, int spineIndex) {
    return _okHandle(_nextHandle++);
  }

  @override
  PageletNativeHandleResult layoutSessionCreate(
    int chapter,
    PageletNativeLayoutOptions options,
  ) {
    return _okHandle(_nextHandle++);
  }

  @override
  PageletNativeLayoutResult layoutRequest(
    int layout,
    int startPage,
    int maxPages,
  ) {
    return PageletNativeLayoutResult(
      status: PageletStatus.ok,
      statusCode: 0,
      internalErrorId: 0,
      stateCode: PageletLayoutState.complete.index,
      requestHandle: 0,
      bytes: Uint8List(0),
    );
  }

  @override
  PageletNativeLayoutResult layoutSubmitMeasurements(
    int request,
    Uint8List measuredBatch,
  ) {
    return layoutRequest(0, 0, 1);
  }

  @override
  PageletNativeHitTestResult hitTest(
    int layout,
    int pageIndex,
    int x,
    int y,
  ) {
    hitTests.add((layout, pageIndex, x, y));
    return hitTestResult ??
        const PageletNativeHitTestResult(
          status: PageletStatus.ok,
          statusCode: 0,
          internalErrorId: 0,
          found: false,
          affinity: 0,
          nodeId: 0,
          utf8ByteOffset: 0,
          fragmentId: 0,
        );
  }

  @override
  PageletNativeStatusResult requestCancel(int request) => okStatus;

  @override
  int debugLiveBufferCount() => 0;

  @override
  PageletNativeResourceResult resourceRead(int book, int resourceId) {
    resourceReads.add((book, resourceId));
    return PageletNativeResourceResult(
      status: PageletStatus.ok,
      statusCode: 0,
      internalErrorId: 0,
      resourceId: resourceId + resourceIdOffset,
      bytes: Uint8List.fromList(<int>[1, 2, 3]),
      path: Uint8List.fromList(utf8.encode('EPUB/images/cover.png')),
      mediaType: Uint8List.fromList(utf8.encode('image/png')),
    );
  }

  @override
  PageletNativeStatusResult handleDispose(int handle) {
    disposedHandles.add(handle);
    if (_disposeResults.isEmpty) {
      return okStatus;
    }
    return _disposeResults.removeAt(0);
  }

  PageletNativeHandleResult _okHandle(int handle) {
    return PageletNativeHandleResult(
      status: PageletStatus.ok,
      statusCode: 0,
      internalErrorId: 0,
      handle: handle,
    );
  }

  static const _summaryJson = '''
{
  "rootfile": "EPUB/package.opf",
  "package_version": "3.0",
  "identifier": "fixture-id",
  "title": "Fixture Book",
  "language": "en",
  "spine": [{"idref": "chapter", "linear": true}],
  "navigation": {
    "source": "epub3-nav",
    "toc": [{
      "label": "Chapter 1",
      "href": "chapter.xhtml",
      "children": [{"label": "Note", "href": "note.xhtml", "children": []}]
    }],
    "page_list": [],
    "landmarks": []
  },
  "diagnostics": []
}
''';
}
