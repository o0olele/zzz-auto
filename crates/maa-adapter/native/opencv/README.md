# OpenCV build headers

Unmodified headers from OpenCV 4.12.0, commit
`49486f61fb25722cbcf586b7f4320921d46fb38e`:
https://github.com/opencv/opencv/tree/4.12.0

Only the transitive includes of `opencv2/core.hpp` and `opencv2/imgproc.hpp`
are included. SHA-256 values are in `headers.sha256.json`; upstream license
and per-file copyright notices are retained.

`config/opencv2/opencv_modules.hpp` is the local consumer configuration for
the two included modules; it is not an upstream source header.

These are build inputs for a small C ABI bridge. The runtime continues to use
the existing, hash-checked `opencv_world4_maa.dll` (4.12.0) from the pinned SDK.
No additional OpenCV runtime is built or distributed. The bridge receives
three function addresses from the existing SDK module after path, hash and
version checks. Mat construction/destruction and matching all use that module;
there is no implicit OpenCV import before the SDK loader runs.
