#ifndef PAGELET_H
#define PAGELET_H

#include <stddef.h>
#include <stdint.h>

#if defined(__cplusplus)
extern "C" {
#endif

typedef uint32_t PageletStatus;
enum {
  PAGELET_STATUS_OK = 0,
  PAGELET_STATUS_INVALID_ARGUMENT = 1,
  PAGELET_STATUS_IO = 2,
  PAGELET_STATUS_INVALID_CONTAINER = 3,
  PAGELET_STATUS_INVALID_PACKAGE = 4,
  PAGELET_STATUS_UNSUPPORTED_FEATURE = 5,
  PAGELET_STATUS_RESOURCE_LIMIT_EXCEEDED = 6,
  PAGELET_STATUS_PARSE = 7,
  PAGELET_STATUS_LAYOUT = 8,
  PAGELET_STATUS_CANCELLED = 9,
  PAGELET_STATUS_PROTOCOL = 10,
  PAGELET_STATUS_INTERNAL = 11,
  PAGELET_STATUS_BUFFER_TOO_SMALL = 12,
};

typedef uint32_t PageletLayoutState;
enum {
  PAGELET_LAYOUT_COMPLETE = 0,
  PAGELET_LAYOUT_NEED_MEASUREMENTS = 1,
  PAGELET_LAYOUT_PAGES = 2,
};

typedef struct PageletByteSlice {
  const uint8_t *data;
  size_t len;
} PageletByteSlice;

typedef struct PageletMutableByteSlice {
  uint8_t *data;
  size_t len;
} PageletMutableByteSlice;

typedef struct PageletNativeBuffer {
  uint64_t id;
  const uint8_t *data;
  size_t len;
} PageletNativeBuffer;

typedef struct PageletLayoutOptions {
  int64_t viewport_width;
  int64_t viewport_height;
  int64_t margin_start;
  int64_t margin_end;
  int64_t margin_top;
  int64_t margin_bottom;
  uint32_t max_pages;
} PageletLayoutOptions;

typedef struct PageletPageRequest {
  uint64_t start_page;
  uint64_t max_pages;
} PageletPageRequest;

typedef struct PageletStatusResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
} PageletStatusResult;

typedef struct PageletHandleResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  uint64_t handle;
} PageletHandleResult;

typedef struct PageletBufferResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  PageletNativeBuffer buffer;
} PageletBufferResult;

typedef struct PageletLayoutResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  PageletLayoutState state;
  uint32_t _state_padding;
  uint64_t request;
  PageletNativeBuffer buffer;
} PageletLayoutResult;

typedef struct PageletResourceResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  uint32_t resource_id;
  uint32_t _resource_padding;
  PageletNativeBuffer bytes;
  PageletNativeBuffer path;
  PageletNativeBuffer media_type;
} PageletResourceResult;

typedef struct PageletHitTestResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  uint8_t found;
  uint8_t affinity;
  uint8_t _padding[6];
  uint32_t node_id;
  uint32_t utf8_byte_offset;
  uint32_t fragment_id;
  uint32_t _tail_padding;
} PageletHitTestResult;

typedef struct PageletAnchorResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  uint8_t found;
  uint8_t _padding[3];
  uint32_t page_index;
} PageletAnchorResult;

typedef struct PageletCopyResult {
  PageletStatus status;
  uint32_t _status_padding;
  uint64_t internal_error_id;
  size_t written;
  size_t required;
} PageletCopyResult;

#if UINTPTR_MAX == UINT64_MAX
#if defined(__cplusplus)
static_assert(sizeof(PageletNativeBuffer) == 24, "PageletNativeBuffer ABI");
static_assert(sizeof(PageletStatusResult) == 16, "PageletStatusResult ABI");
static_assert(sizeof(PageletHandleResult) == 24, "PageletHandleResult ABI");
static_assert(sizeof(PageletBufferResult) == 40, "PageletBufferResult ABI");
static_assert(sizeof(PageletLayoutResult) == 56, "PageletLayoutResult ABI");
static_assert(sizeof(PageletResourceResult) == 96, "PageletResourceResult ABI");
static_assert(sizeof(PageletHitTestResult) == 40, "PageletHitTestResult ABI");
static_assert(sizeof(PageletAnchorResult) == 24, "PageletAnchorResult ABI");
static_assert(sizeof(PageletCopyResult) == 32, "PageletCopyResult ABI");
#else
_Static_assert(sizeof(PageletNativeBuffer) == 24, "PageletNativeBuffer ABI");
_Static_assert(sizeof(PageletStatusResult) == 16, "PageletStatusResult ABI");
_Static_assert(sizeof(PageletHandleResult) == 24, "PageletHandleResult ABI");
_Static_assert(sizeof(PageletBufferResult) == 40, "PageletBufferResult ABI");
_Static_assert(sizeof(PageletLayoutResult) == 56, "PageletLayoutResult ABI");
_Static_assert(sizeof(PageletResourceResult) == 96,
               "PageletResourceResult ABI");
_Static_assert(sizeof(PageletHitTestResult) == 40,
               "PageletHitTestResult ABI");
_Static_assert(sizeof(PageletAnchorResult) == 24, "PageletAnchorResult ABI");
_Static_assert(sizeof(PageletCopyResult) == 32, "PageletCopyResult ABI");
#endif
#endif

PageletHandleResult pagelet_engine_create(void);
PageletHandleResult pagelet_book_open_path(uint64_t engine,
                                           PageletByteSlice path);
PageletHandleResult pagelet_book_open_fd(uint64_t engine, int32_t fd);
PageletBufferResult pagelet_book_summary(uint64_t book);
PageletBufferResult pagelet_book_navigation(uint64_t book);
PageletHandleResult pagelet_chapter_open(uint64_t book,
                                         uint64_t spine_index);
PageletHandleResult pagelet_layout_session_create(
    uint64_t chapter, PageletLayoutOptions options);
PageletLayoutResult pagelet_layout_request(uint64_t layout,
                                           PageletPageRequest request);
PageletLayoutResult pagelet_layout_submit_measurements(
    uint64_t request, PageletByteSlice measured);
PageletResourceResult pagelet_resource_read(uint64_t book,
                                            uint32_t resource_id);
PageletHitTestResult pagelet_hit_test(uint64_t layout, uint32_t page_index,
                                      int64_t x, int64_t y);
PageletAnchorResult pagelet_anchor_to_page(
    uint64_t layout, uint32_t document_id, uint32_t node_id,
    uint32_t utf8_byte_offset, uint32_t affinity);
PageletStatusResult pagelet_request_cancel(uint64_t request);
PageletStatusResult pagelet_handle_dispose(uint64_t handle);

PageletCopyResult pagelet_buffer_copy(PageletNativeBuffer buffer,
                                      PageletMutableByteSlice destination);
PageletStatusResult pagelet_buffer_free(PageletNativeBuffer buffer);
size_t pagelet_debug_live_buffer_count(void);

/*
 * Buffer ownership:
 * - A successful buffer-bearing call transfers one Rust-owned descriptor.
 * - data is read-only and valid until pagelet_buffer_free is called.
 * - pagelet_buffer_copy copies into host-owned memory without transferring it.
 * - pagelet_buffer_free is idempotent for an already-released descriptor.
 *
 * Wire ownership:
 * - layout_request returns a versioned MeasureBatch in buffer.
 * - layout_submit_measurements accepts a versioned MeasuredBatch.
 * - PAGELET_LAYOUT_PAGES returns a versioned pageletScene PageBatch.
 */

#if defined(__cplusplus)
}
#endif

#endif
