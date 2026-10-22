"""Small driver/RAM probe. Works before PyTorch or any model is installed."""
import ctypes
import os


def memory_bytes():
    try:
        if os.name == "nt":
            class Memory(ctypes.Structure):
                _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
                    (name, ctypes.c_ulonglong) for name in
                    ("total", "available", "page_total", "page_available", "virtual", "virtual_available", "extended")]
            status = Memory()
            status.length = ctypes.sizeof(status)
            if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
                return status.total
        elif hasattr(os, "sysconf"):
            return os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError, AttributeError):
        pass
    return 0


def cuda_device():
    try:
        cuda = ctypes.WinDLL("nvcuda.dll") if os.name == "nt" else ctypes.CDLL("libcuda.so.1")
        version, count, device = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
        if cuda.cuDriverGetVersion(ctypes.byref(version)) or cuda.cuInit(0):
            return {}
        if cuda.cuDeviceGetCount(ctypes.byref(count)) or count.value < 1 or cuda.cuDeviceGet(ctypes.byref(device), 0):
            return {}
        name, memory = ctypes.create_string_buffer(256), ctypes.c_size_t()
        major, minor = ctypes.c_int(), ctypes.c_int()
        if (cuda.cuDeviceGetName(name, len(name), device)
                or cuda.cuDeviceTotalMem_v2(ctypes.byref(memory), device)
                or cuda.cuDeviceGetAttribute(ctypes.byref(major), 75, device)
                or cuda.cuDeviceGetAttribute(ctypes.byref(minor), 76, device)):
            return {}
        return dict(cuda_version=version.value, gpu_name=name.value.decode("utf-8", errors="replace"),
                    vram_bytes=memory.value, compute=major.value * 10 + minor.value)
    except (OSError, AttributeError):
        return {}


def probe():
    result = dict(ram_bytes=memory_bytes(), cuda_version=0, gpu_name="", vram_bytes=0, compute=0)
    result.update(cuda_device())
    # CUDA 13 removed older GPU architectures. CUDA 13.x supports minor-version
    # compatibility with a CUDA 13.0 driver; PyTorch is pinned to cu128.
    supported = result["compute"] >= 75
    result["speech_gpu"] = supported and result["cuda_version"] >= 12080
    result["text_gpu"] = supported and result["cuda_version"] >= 13000
    return result
