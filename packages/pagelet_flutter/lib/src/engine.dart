import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'errors.dart';
import 'native_api.dart';

part 'book_session.dart';
part 'resource_loader.dart';

/// Owns one native pagelet engine and every book opened under it.
final class PageletEngine {
  /// Creates a native engine using an explicit library, an explicit path, or
  /// the default pagelet library for the current platform.
  factory PageletEngine({DynamicLibrary? library, String? libraryPath}) {
    return PageletEngine._fromNativeApi(
      FfiPageletNativeApi.open(library: library, libraryPath: libraryPath),
    );
  }

  PageletEngine._fromNativeApi(PageletNativeApi nativeApi)
      : _nativeApi = nativeApi,
        _handle = _requireHandle(
          nativeApi,
          nativeApi.engineCreate(),
          'engine_create',
        );

  final PageletNativeApi _nativeApi;
  final int _handle;
  final Set<BookSession> _books = <BookSession>{};
  bool _isDisposed = false;

  /// Whether this wrapper and its native descendants have been disposed.
  bool get isDisposed => _isDisposed;

  /// Opens an EPUB from a filesystem path.
  BookSession openBook(String path) {
    _ensureOpen();
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    final handle = _requireHandle(
      _nativeApi,
      _nativeApi.bookOpenPath(_handle, path),
      'book_open_path',
    );
    return _registerBook(handle);
  }

  /// Opens an EPUB from a borrowed POSIX file descriptor.
  ///
  /// Pagelet clones and reads the descriptor without adopting or closing it.
  BookSession openBookFileDescriptor(int fd) {
    _ensureOpen();
    if (fd < 0) {
      throw ArgumentError.value(fd, 'fd', 'must be non-negative');
    }
    final handle = _requireHandle(
      _nativeApi,
      _nativeApi.bookOpenFileDescriptor(_handle, fd),
      'book_open_fd',
    );
    return _registerBook(handle);
  }

  /// Disposes the engine and atomically invalidates its book wrappers.
  ///
  /// Native engine disposal also releases all descendant handles. Repeated
  /// calls are safe and do not cross the FFI boundary again.
  void dispose() {
    if (_isDisposed) {
      return;
    }
    _requireSuccess(
      _nativeApi.handleDispose(_handle),
      'handle_dispose(engine)',
    );
    _isDisposed = true;
    for (final book in _books.toList(growable: false)) {
      book._markDisposedByOwner();
    }
    _books.clear();
  }

  BookSession _registerBook(int handle) {
    final book = BookSession._(this, handle);
    _books.add(book);
    return book;
  }

  void _disposeBook(BookSession book) {
    if (book._isDisposed) {
      return;
    }
    if (_isDisposed) {
      book._markDisposedByOwner();
      return;
    }
    _requireSuccess(
      _nativeApi.handleDispose(book._handle),
      'handle_dispose(book)',
    );
    book._markDisposedByOwner();
    _books.remove(book);
  }

  void _ensureOpen() {
    if (_isDisposed) {
      throw StateError('PageletEngine has been disposed.');
    }
  }
}

/// Internal seam for lifecycle contract tests.
///
/// This type is deliberately omitted from the package facade so application
/// code remains coupled to [PageletEngine], not the native transport.
final class PageletEngineTesting {
  const PageletEngineTesting._();

  /// Creates an engine over a custom native API.
  static PageletEngine create(PageletNativeApi nativeApi) {
    return PageletEngine._fromNativeApi(nativeApi);
  }
}

int _requireHandle(
  PageletNativeApi nativeApi,
  PageletNativeHandleResult result,
  String operation,
) {
  if (result.status == PageletStatus.ok && result.handle != 0) {
    return result.handle;
  }
  if (result.handle != 0) {
    nativeApi.handleDispose(result.handle);
  }
  if (result.status == PageletStatus.ok) {
    throw PageletException(
      operation: operation,
      status: PageletStatus.protocol,
      statusCode: PageletStatus.protocol.code,
      internalErrorId: 0,
      message: 'native operation returned a zero handle',
    );
  }
  throw PageletException(
    operation: operation,
    status: result.status,
    statusCode: result.statusCode,
    internalErrorId: result.internalErrorId,
  );
}

void _requireSuccess(PageletNativeStatusResult result, String operation) {
  if (result.status == PageletStatus.ok) {
    return;
  }
  throw PageletException(
    operation: operation,
    status: result.status,
    statusCode: result.statusCode,
    internalErrorId: result.internalErrorId,
  );
}
