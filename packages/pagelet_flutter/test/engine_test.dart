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
  });
}

final class _FakeNativeApi implements PageletNativeApi {
  _FakeNativeApi({
    this.openPathResult,
    List<PageletNativeStatusResult>? disposeResults,
  }) : _disposeResults = disposeResults ?? <PageletNativeStatusResult>[];

  static const okStatus = PageletNativeStatusResult(
    status: PageletStatus.ok,
    statusCode: 0,
    internalErrorId: 0,
  );

  int _nextHandle = 1;
  final PageletNativeHandleResult? openPathResult;
  final List<PageletNativeStatusResult> _disposeResults;
  final List<int> createdEngineHandles = <int>[];
  final List<(int, String)> openedPaths = <(int, String)>[];
  final List<(int, int)> openedFileDescriptors = <(int, int)>[];
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
}
