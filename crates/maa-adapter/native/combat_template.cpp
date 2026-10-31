#include <opencv2/core.hpp>
#include <opencv2/core/version.hpp>
#include <opencv2/imgproc.hpp>
#include <cmath>
#include <cstdio>
#include <exception>
#include <new>

static_assert(CV_VERSION_MAJOR == 4 && CV_VERSION_MINOR == 12 && CV_VERSION_REVISION == 0);
namespace {
using MatConstructor = void (*)(cv::Mat*, int, int, int, void*, size_t);
using MatDestructor = void (*)(cv::Mat*);
struct Api {
    MatConstructor construct;
    MatDestructor destroy;
    decltype(&cv::matchTemplate) match;
};
Api api{};

// Mat 布局由同版本官方头文件确定，构造/销毁都交给同一已校验 DLL。
// 不链接 import lib，不在 main/load_sdk 之前隐式加载 OpenCV。
class BorrowedMat {
    alignas(cv::Mat) unsigned char storage[sizeof(cv::Mat)];
public:
    BorrowedMat(int rows, int cols, int type, void* data, size_t stride = 0) {
        api.construct(reinterpret_cast<cv::Mat*>(storage), rows, cols, type, data, stride);
    }
    ~BorrowedMat() { api.destroy(&get()); }
    BorrowedMat(const BorrowedMat&) = delete;
    BorrowedMat& operator=(const BorrowedMat&) = delete;
    cv::Mat& get() { return *std::launder(reinterpret_cast<cv::Mat*>(storage)); }
};
}

// Rust OnceLock 在发布给工作线程前完成初始化；函数表之后保持只读。
extern "C" void zzz_combat_opencv_init(void* construct, void* destroy, void* match) noexcept {
    api = {reinterpret_cast<MatConstructor>(construct), reinterpret_cast<MatDestructor>(destroy),
           reinterpret_cast<decltype(&cv::matchTemplate)>(match)};
}

// 只借用 Rust 缓冲区，同步返回；不跨 ABI 传递 Mat、STL 或 C++ 异常。
extern "C" int zzz_combat_match(
    const unsigned char* image, int width, int height, size_t stride,
    const unsigned char* templ, int tw, int th, const unsigned char* mask,
    float* scratch, double* score, char* error, size_t error_capacity) noexcept {
    try {
        BorrowedMat source(height, width, CV_8UC3, const_cast<unsigned char*>(image), stride);
        BorrowedMat target(th, tw, CV_8UC3, const_cast<unsigned char*>(templ));
        BorrowedMat result(height - th + 1, width - tw + 1, CV_32FC1, scratch);
        if (mask) {
            BorrowedMat mask_mat(th, tw, CV_8UC1, const_cast<unsigned char*>(mask));
            api.match(source.get(), target.get(), result.get(), cv::TM_CCOEFF_NORMED, mask_mat.get());
        } else {
            const cv::_InputArray empty;
            api.match(source.get(), target.get(), result.get(), cv::TM_CCOEFF_NORMED, empty);
        }
        // 与原路径一致：忽略 NaN/Inf；最高分初值为 0，负分不能成为命中。
        bool finite = false;
        double best = 0;
        for (int y = 0; y < height - th + 1; ++y) {
            const float* row = scratch + y * (width - tw + 1);
            for (int x = 0; x < width - tw + 1; ++x) {
                if (std::isfinite(row[x])) {
                    finite = true;
                    if (row[x] > best) best = row[x];
                }
            }
        }
        *score = best;
        // 原路径 NMS 会删除小于 0.1f 的所有候选，即使用户设置更低阈值也不命中。
        return finite && best >= 0.1f ? 1 : 0;
    } catch (const std::exception& exception) {
        std::snprintf(error, error_capacity, "%s", exception.what());
    } catch (...) {
        std::snprintf(error, error_capacity, "%s", "OpenCV unknown exception");
    }
    return -1;
}
