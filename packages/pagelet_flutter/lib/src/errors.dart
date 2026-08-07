/// Stable status codes returned by the pagelet C ABI.
enum PageletStatus {
  ok(0),
  invalidArgument(1),
  io(2),
  invalidContainer(3),
  invalidPackage(4),
  unsupportedFeature(5),
  resourceLimitExceeded(6),
  parse(7),
  layout(8),
  cancelled(9),
  protocol(10),
  internal(11),
  bufferTooSmall(12),
  unknown(-1);

  const PageletStatus(this.code);

  /// Numeric value used by the C ABI.
  final int code;

  /// Converts a C ABI status code without throwing on a future unknown value.
  static PageletStatus fromCode(int code) {
    for (final status in values) {
      if (status.code == code) {
        return status;
      }
    }
    return unknown;
  }
}

/// Failure returned by a pagelet native operation.
final class PageletException implements Exception {
  /// Creates an exception for one failed native operation.
  const PageletException({
    required this.operation,
    required this.status,
    required this.statusCode,
    required this.internalErrorId,
    this.message,
  });

  /// C ABI operation that failed.
  final String operation;

  /// Stable status category, or [PageletStatus.unknown].
  final PageletStatus status;

  /// Original numeric status returned by the native library.
  final int statusCode;

  /// Correlation identifier for internal failures, otherwise zero.
  final int internalErrorId;

  /// Additional host-side context, when available.
  final String? message;

  @override
  String toString() {
    final detail = message == null ? '' : ': $message';
    final internal =
        internalErrorId == 0 ? '' : ' (internal error $internalErrorId)';
    return 'PageletException[$operation]: ${status.name} '
        '($statusCode)$internal$detail';
  }
}
