import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

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

/// Host-owned resource data copied from native buffers.
final class PageletNativeResourceResult {
  const PageletNativeResourceResult({
    required this.status,
    required this.statusCode,
    required this.internalErrorId,
    required this.resourceId,
    required this.bytes,
    required this.path,
    required this.mediaType,
  });

  final PageletStatus status;
  final int statusCode;
  final int internalErrorId;
  final int resourceId;
  final Uint8List bytes;
  final Uint8List path;
  final Uint8List mediaType;
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

  /// Copies one publication resource into host-owned memory.
  PageletNativeResourceResult resourceRead(int book, int resourceId);

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
        _resourceRead =
            library.lookupFunction<_ResourceReadNative, _ResourceReadDart>(
          'pagelet_resource_read',
        ),
        _bufferCopy =
            library.lookupFunction<_BufferCopyNative, _BufferCopyDart>(
          'pagelet_buffer_copy',
        ),
        _bufferFree =
            library.lookupFunction<_BufferFreeNative, _BufferFreeDart>(
          'pagelet_buffer_free',
        ),
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
  final _ResourceReadDart _resourceRead;
  final _BufferCopyDart _bufferCopy;
  final _BufferFreeDart _bufferFree;
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
  PageletNativeResourceResult resourceRead(int book, int resourceId) {
    final result = _resourceRead(book, resourceId);
    final status = PageletStatus.fromCode(result.status);
    if (status != PageletStatus.ok) {
      return PageletNativeResourceResult(
        status: status,
        statusCode: result.status,
        internalErrorId: result.internalErrorId,
        resourceId: result.resourceId,
        bytes: Uint8List(0),
        path: Uint8List(0),
        mediaType: Uint8List(0),
      );
    }
    try {
      final bytes = _copyBuffer(result.bytes);
      final path = _copyBuffer(result.path);
      final mediaType = _copyBuffer(result.mediaType);
      _releaseBuffers(
        <_NativeBuffer>[result.bytes, result.path, result.mediaType],
        checkStatus: true,
      );
      return PageletNativeResourceResult(
        status: status,
        statusCode: result.status,
        internalErrorId: result.internalErrorId,
        resourceId: result.resourceId,
        bytes: bytes,
        path: path,
        mediaType: mediaType,
      );
    } catch (_) {
      _releaseBuffers(
        <_NativeBuffer>[result.bytes, result.path, result.mediaType],
        checkStatus: false,
      );
      rethrow;
    }
  }

  Uint8List _copyBuffer(_NativeBuffer buffer) {
    if (buffer.id == 0) {
      if (buffer.length != 0 || buffer.data != nullptr) {
        throw const FormatException('Native buffer descriptor is invalid.');
      }
      return Uint8List(0);
    }
    final destination =
        buffer.length == 0 ? nullptr : calloc<Uint8>(buffer.length);
    final slice = calloc<_NativeMutableByteSlice>()
      ..ref.data = destination
      ..ref.length = buffer.length;
    try {
      final result = _bufferCopy(buffer, slice.ref);
      if (PageletStatus.fromCode(result.status) != PageletStatus.ok ||
          result.written != buffer.length ||
          result.required != buffer.length) {
        throw PageletException(
          operation: 'buffer_copy',
          status: PageletStatus.fromCode(result.status),
          statusCode: result.status,
          internalErrorId: result.internalErrorId,
        );
      }
      return result.written == 0
          ? Uint8List(0)
          : Uint8List.fromList(destination.asTypedList(result.written));
    } finally {
      if (destination != nullptr) {
        calloc.free(destination);
      }
      calloc.free(slice);
    }
  }

  void _releaseBuffers(
    List<_NativeBuffer> buffers, {
    required bool checkStatus,
  }) {
    PageletNativeStatusResult? firstFailure;
    for (final buffer in buffers) {
      final result = _statusResult(_bufferFree(buffer));
      if (result.status != PageletStatus.ok) {
        firstFailure ??= result;
      }
    }
    if (checkStatus && firstFailure != null) {
      throw PageletException(
        operation: 'buffer_free',
        status: firstFailure.status,
        statusCode: firstFailure.statusCode,
        internalErrorId: firstFailure.internalErrorId,
      );
    }
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

final class _NativeMutableByteSlice extends Struct {
  external Pointer<Uint8> data;

  @Size()
  external int length;
}

final class _NativeBuffer extends Struct {
  @Uint64()
  external int id;

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

final class _NativeResourceResult extends Struct {
  @Uint32()
  external int status;

  @Uint32()
  external int statusPadding;

  @Uint64()
  external int internalErrorId;

  @Uint32()
  external int resourceId;

  @Uint32()
  external int resourcePadding;

  external _NativeBuffer bytes;
  external _NativeBuffer path;
  external _NativeBuffer mediaType;
}

final class _NativeCopyResult extends Struct {
  @Uint32()
  external int status;

  @Uint32()
  external int statusPadding;

  @Uint64()
  external int internalErrorId;

  @Size()
  external int written;

  @Size()
  external int required;
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
typedef _ResourceReadNative = _NativeResourceResult Function(
    Uint64 book, Uint32 resourceId);
typedef _ResourceReadDart = _NativeResourceResult Function(
    int book, int resourceId);
typedef _BufferCopyNative = _NativeCopyResult Function(
    _NativeBuffer buffer, _NativeMutableByteSlice destination);
typedef _BufferCopyDart = _NativeCopyResult Function(
    _NativeBuffer buffer, _NativeMutableByteSlice destination);
typedef _BufferFreeNative = _NativeStatusResult Function(_NativeBuffer buffer);
typedef _BufferFreeDart = _NativeStatusResult Function(_NativeBuffer buffer);
typedef _HandleDisposeNative = _NativeStatusResult Function(Uint64 handle);
typedef _HandleDisposeDart = _NativeStatusResult Function(int handle);
