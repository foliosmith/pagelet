import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'errors.dart';

/// Host-neutral result carrying an opaque pagelet handle.
final class PageletNativeHandleResult {
  /// Creates a handle result translated from the C ABI.
  const PageletNativeHandleResult({
    required this.status,
    required this.statusCode,
    required this.internalErrorId,
    required this.handle,
  });

  /// Stable status category.
  final PageletStatus status;

  /// Original numeric status returned by the native library.
  final int statusCode;

  /// Correlation identifier for internal failures, otherwise zero.
  final int internalErrorId;

  /// Generation-safe opaque handle, or zero on failure.
  final int handle;
}

/// Host-neutral result carrying only a pagelet status.
final class PageletNativeStatusResult {
  /// Creates a status result translated from the C ABI.
  const PageletNativeStatusResult({
    required this.status,
    required this.statusCode,
    required this.internalErrorId,
  });

  /// Stable status category.
  final PageletStatus status;

  /// Original numeric status returned by the native library.
  final int statusCode;

  /// Correlation identifier for internal failures, otherwise zero.
  final int internalErrorId;
}

/// Narrow native surface required by the engine and book wrappers.
///
/// The interface is intentionally internal to the package facade. It also
/// keeps lifecycle behavior independently testable without loading a dynamic
/// library.
abstract interface class PageletNativeApi {
  /// Creates a native engine.
  PageletNativeHandleResult engineCreate();

  /// Opens an EPUB from a UTF-8 path under [engine].
  PageletNativeHandleResult bookOpenPath(int engine, String path);

  /// Opens an EPUB from a borrowed POSIX file descriptor.
  PageletNativeHandleResult bookOpenFileDescriptor(int engine, int fd);

  /// Idempotently disposes a handle and its native descendants.
  PageletNativeStatusResult handleDispose(int handle);
}

/// C ABI implementation of [PageletNativeApi].
final class FfiPageletNativeApi implements PageletNativeApi {
  /// Resolves the symbols needed by the first Flutter adapter slice.
  FfiPageletNativeApi(DynamicLibrary library)
      : _engineCreate =
            library.lookupFunction<_EngineCreateNative, _EngineCreateDart>(
          'pagelet_engine_create',
        ),
        _bookOpenPath =
            library.lookupFunction<_BookOpenPathNative, _BookOpenPathDart>(
          'pagelet_book_open_path',
        ),
        _bookOpenFileDescriptor = library.lookupFunction<
            _BookOpenFileDescriptorNative,
            _BookOpenFileDescriptorDart>('pagelet_book_open_fd'),
        _handleDispose =
            library.lookupFunction<_HandleDisposeNative, _HandleDisposeDart>(
          'pagelet_handle_dispose',
        );

  /// Opens the explicitly supplied [library], [libraryPath], or the default
  /// library name for the current native platform.
  factory FfiPageletNativeApi.open({
    DynamicLibrary? library,
    String? libraryPath,
  }) {
    if (library != null && libraryPath != null) {
      throw ArgumentError('Provide either library or libraryPath, not both.');
    }
    return FfiPageletNativeApi(library ?? _openDynamicLibrary(libraryPath));
  }

  final _EngineCreateDart _engineCreate;
  final _BookOpenPathDart _bookOpenPath;
  final _BookOpenFileDescriptorDart _bookOpenFileDescriptor;
  final _HandleDisposeDart _handleDispose;

  @override
  PageletNativeHandleResult engineCreate() {
    return _handleResult(_engineCreate());
  }

  @override
  PageletNativeHandleResult bookOpenPath(int engine, String path) {
    final pathBytes = utf8.encode(path);
    final slicePointer = calloc<_NativeByteSlice>();
    final dataPointer =
        pathBytes.isEmpty ? nullptr : calloc<Uint8>(pathBytes.length);
    if (pathBytes.isNotEmpty) {
      dataPointer.asTypedList(pathBytes.length).setAll(0, pathBytes);
    }
    slicePointer.ref
      ..data = dataPointer
      ..length = pathBytes.length;
    try {
      return _handleResult(_bookOpenPath(engine, slicePointer.ref));
    } finally {
      if (dataPointer != nullptr) {
        calloc.free(dataPointer);
      }
      calloc.free(slicePointer);
    }
  }

  @override
  PageletNativeHandleResult bookOpenFileDescriptor(int engine, int fd) {
    return _handleResult(_bookOpenFileDescriptor(engine, fd));
  }

  @override
  PageletNativeStatusResult handleDispose(int handle) {
    return _statusResult(_handleDispose(handle));
  }
}

DynamicLibrary _openDynamicLibrary(String? libraryPath) {
  if (libraryPath != null) {
    if (libraryPath.isEmpty) {
      throw ArgumentError.value(
        libraryPath,
        'libraryPath',
        'must not be empty',
      );
    }
    return DynamicLibrary.open(libraryPath);
  }
  if (Platform.isIOS) {
    return DynamicLibrary.process();
  }
  if (Platform.isMacOS) {
    return DynamicLibrary.open('libpagelet.dylib');
  }
  if (Platform.isAndroid || Platform.isLinux) {
    return DynamicLibrary.open('libpagelet.so');
  }
  if (Platform.isWindows) {
    return DynamicLibrary.open('pagelet.dll');
  }
  throw UnsupportedError(
    'pagelet_flutter does not support ${Platform.operatingSystem}',
  );
}

PageletNativeHandleResult _handleResult(_NativeHandleResult result) {
  return PageletNativeHandleResult(
    status: PageletStatus.fromCode(result.status),
    statusCode: result.status,
    internalErrorId: result.internalErrorId,
    handle: result.handle,
  );
}

PageletNativeStatusResult _statusResult(_NativeStatusResult result) {
  return PageletNativeStatusResult(
    status: PageletStatus.fromCode(result.status),
    statusCode: result.status,
    internalErrorId: result.internalErrorId,
  );
}

final class _NativeByteSlice extends Struct {
  external Pointer<Uint8> data;

  @Size()
  external int length;
}

final class _NativeStatusResult extends Struct {
  @Uint32()
  external int status;

  @Uint32()
  external int statusPadding;

  @Uint64()
  external int internalErrorId;
}

final class _NativeHandleResult extends Struct {
  @Uint32()
  external int status;

  @Uint32()
  external int statusPadding;

  @Uint64()
  external int internalErrorId;

  @Uint64()
  external int handle;
}

typedef _EngineCreateNative = _NativeHandleResult Function();
typedef _EngineCreateDart = _NativeHandleResult Function();
typedef _BookOpenPathNative = _NativeHandleResult Function(
    Uint64 engine, _NativeByteSlice path);
typedef _BookOpenPathDart = _NativeHandleResult Function(
    int engine, _NativeByteSlice path);
typedef _BookOpenFileDescriptorNative = _NativeHandleResult Function(
    Uint64 engine, Int32 fd);
typedef _BookOpenFileDescriptorDart = _NativeHandleResult Function(
    int engine, int fd);
typedef _HandleDisposeNative = _NativeStatusResult Function(Uint64 handle);
typedef _HandleDisposeDart = _NativeStatusResult Function(int handle);
