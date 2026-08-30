part of 'engine.dart';

const int _maximumUint32 = 0xffffffff;
const int _maximumPageIndex = 0x7fffffffffffffff;
const int _minimumInt64 = -0x8000000000000000;
const int _maximumInt64 = 0x7fffffffffffffff;

final class PageletLayoutOptions {
  const PageletLayoutOptions({
    required this.viewportWidth,
    required this.viewportHeight,
    this.marginStart = 0,
    this.marginEnd = 0,
    this.marginTop = 0,
    this.marginBottom = 0,
    this.maxPages = 1,
  });

  final double viewportWidth;
  final double viewportHeight;
  final double marginStart;
  final double marginEnd;
  final double marginTop;
  final double marginBottom;
  final int maxPages;
}

final class PageletPageRequest {
  const PageletPageRequest({this.startPage = 0, this.maxPages = 1});

  final int startPage;
  final int maxPages;
}

enum PageletLayoutState { complete, needMeasurements, pages }

final class PageletLayoutResult {
  PageletLayoutResult({
    required this.state,
    required this.requestHandle,
    required Uint8List bytes,
  }) : bytes = Uint8List.fromList(bytes);

  final PageletLayoutState state;
  final int requestHandle;
  final Uint8List bytes;
}

final class PageletHitTestResult {
  const PageletHitTestResult({
    required this.nodeId,
    required this.utf8ByteOffset,
    required this.fragmentId,
    required this.affinity,
  });

  final int nodeId;
  final int utf8ByteOffset;
  final int fragmentId;
  final PageTextAffinity affinity;
}

/// Owns one native layout session under a [ChapterSession].
final class LayoutSession {
  LayoutSession._(this._chapter, this._handle);

  final ChapterSession _chapter;
  final int _handle;
  bool _isDisposed = false;

  bool get isDisposed => _isDisposed;

  PageletLayoutResult requestPages(PageletPageRequest request) {
    _ensureOpen();
    _requirePageRequest(request);
    return _layoutResult(
      _chapter._book._engine._nativeApi.layoutRequest(
        _handle,
        request.startPage,
        request.maxPages,
      ),
      'layout_request',
    );
  }

  PageletLayoutResult submitMeasurements(
    int requestHandle,
    Uint8List measuredBatch,
  ) {
    _ensureOpen();
    if (requestHandle <= 0) {
      throw RangeError.value(
        requestHandle,
        'requestHandle',
        'must be positive',
      );
    }
    return _layoutResult(
      _chapter._book._engine._nativeApi.layoutSubmitMeasurements(
        requestHandle,
        measuredBatch,
      ),
      'layout_submit_measurements',
    );
  }

  PageletHitTestResult? hitTest(int pageIndex, Offset position) {
    _ensureOpen();
    if (pageIndex < 0 || pageIndex > _maximumUint32) {
      throw RangeError.range(pageIndex, 0, _maximumUint32, 'pageIndex');
    }
    final result = _chapter._book._engine._nativeApi.hitTest(
      _handle,
      pageIndex,
      _layoutUnitRaw(position.dx, 'x'),
      _layoutUnitRaw(position.dy, 'y'),
    );
    if (result.status != PageletStatus.ok) {
      throw PageletException(
        operation: 'hit_test',
        status: result.status,
        statusCode: result.statusCode,
        internalErrorId: result.internalErrorId,
      );
    }
    if (!result.found) {
      return null;
    }
    if (result.affinity < 0 ||
        result.affinity >= PageTextAffinity.values.length) {
      throw FormatException(
        'Native text affinity ${result.affinity} is invalid.',
      );
    }
    return PageletHitTestResult(
      nodeId: result.nodeId,
      utf8ByteOffset: result.utf8ByteOffset,
      fragmentId: result.fragmentId,
      affinity: PageTextAffinity.values[result.affinity],
    );
  }

  void cancelRequest(int requestHandle) {
    _ensureOpen();
    if (requestHandle <= 0) {
      throw RangeError.value(
        requestHandle,
        'requestHandle',
        'must be positive',
      );
    }
    _requireSuccess(
      _chapter._book._engine._nativeApi.requestCancel(requestHandle),
      'request_cancel',
    );
  }

  void dispose() => _chapter._disposeLayout(this);

  void _markDisposedByOwner() {
    _isDisposed = true;
  }

  void _ensureOpen() {
    if (_isDisposed) {
      throw StateError('LayoutSession has been disposed.');
    }
  }
}

PageletLayoutResult _layoutResult(
  PageletNativeLayoutResult result,
  String operation,
) {
  if (result.status != PageletStatus.ok) {
    throw PageletException(
      operation: operation,
      status: result.status,
      statusCode: result.statusCode,
      internalErrorId: result.internalErrorId,
    );
  }
  if (result.stateCode < 0 ||
      result.stateCode >= PageletLayoutState.values.length) {
    throw FormatException(
        'Native layout state ${result.stateCode} is invalid.');
  }
  final state = PageletLayoutState.values[result.stateCode];
  if (state == PageletLayoutState.needMeasurements &&
      (result.requestHandle == 0 || result.bytes.isEmpty)) {
    throw const FormatException('Measurement request is missing its payload.');
  }
  if (state == PageletLayoutState.pages && result.bytes.isEmpty) {
    throw const FormatException('Page result is missing its payload.');
  }
  return PageletLayoutResult(
    state: state,
    requestHandle: result.requestHandle,
    bytes: result.bytes,
  );
}

PageletNativeLayoutOptions _nativeLayoutOptions(PageletLayoutOptions options) {
  if (options.viewportWidth <= 0 || options.viewportHeight <= 0) {
    throw ArgumentError('Viewport dimensions must be positive.');
  }
  if (options.marginStart < 0 ||
      options.marginEnd < 0 ||
      options.marginTop < 0 ||
      options.marginBottom < 0) {
    throw ArgumentError('Layout margins must be non-negative.');
  }
  if (options.maxPages <= 0 || options.maxPages > _maximumUint32) {
    throw RangeError.range(options.maxPages, 1, _maximumUint32, 'maxPages');
  }
  return PageletNativeLayoutOptions(
    viewportWidth: _layoutUnitRaw(options.viewportWidth, 'viewportWidth'),
    viewportHeight: _layoutUnitRaw(options.viewportHeight, 'viewportHeight'),
    marginStart: _layoutUnitRaw(options.marginStart, 'marginStart'),
    marginEnd: _layoutUnitRaw(options.marginEnd, 'marginEnd'),
    marginTop: _layoutUnitRaw(options.marginTop, 'marginTop'),
    marginBottom: _layoutUnitRaw(options.marginBottom, 'marginBottom'),
    maxPages: options.maxPages,
  );
}

int _layoutUnitRaw(double value, String field) {
  if (!value.isFinite) {
    throw ArgumentError.value(value, field, 'must be finite');
  }
  final raw = (value * 64).round();
  if (raw < _minimumInt64 || raw > _maximumInt64) {
    throw RangeError.value(value, field, 'is outside the layout range');
  }
  return raw;
}

void _requirePageRequest(PageletPageRequest request) {
  if (request.startPage < 0 || request.startPage > _maximumPageIndex) {
    throw RangeError.range(
      request.startPage,
      0,
      _maximumPageIndex,
      'startPage',
    );
  }
  if (request.maxPages <= 0 || request.maxPages > _maximumPageIndex) {
    throw RangeError.range(request.maxPages, 1, _maximumPageIndex, 'maxPages');
  }
}
