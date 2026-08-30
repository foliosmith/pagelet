part of 'engine.dart';

/// One publication resource copied into Dart-owned memory.
final class PageletResource {
  PageletResource({
    required this.id,
    required this.path,
    required this.mediaType,
    required Uint8List bytes,
  }) : _bytes = Uint8List.fromList(bytes);

  final int id;
  final String path;
  final String mediaType;
  final Uint8List _bytes;

  Uint8List get bytes => Uint8List.fromList(_bytes);
}

/// Reads image, font, and other EPUB resources only when requested.
final class PageletResourceLoader {
  PageletResourceLoader._(this._book);

  final BookSession _book;

  PageletResource read(int resourceId) {
    _book._ensureOpen();
    if (resourceId < 0 || resourceId > 0xffffffff) {
      throw RangeError.range(resourceId, 0, 0xffffffff, 'resourceId');
    }
    final result = _book._engine._nativeApi.resourceRead(
      _book._handle,
      resourceId,
    );
    if (result.status != PageletStatus.ok) {
      throw PageletException(
        operation: 'resource_read',
        status: result.status,
        statusCode: result.statusCode,
        internalErrorId: result.internalErrorId,
      );
    }
    if (result.resourceId != resourceId) {
      throw const FormatException(
        'Native resource id does not match the requested id.',
      );
    }
    final path = utf8.decode(result.path, allowMalformed: false);
    final mediaType = utf8.decode(result.mediaType, allowMalformed: false);
    if (path.isEmpty || mediaType.isEmpty) {
      throw const FormatException('Native resource metadata is empty.');
    }
    return PageletResource(
      id: result.resourceId,
      path: path,
      mediaType: mediaType,
      bytes: result.bytes,
    );
  }
}
