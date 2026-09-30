/*++ FreeTouch: kernel placement new/delete operators. --*/
#include <wdm.h>

void *__cdecl operator new(
    size_t iSize,
    POOL_FLAGS poolFlags,
    ULONG tag)
{
    PVOID result = ExAllocatePool2(poolFlags, iSize, tag);
    return result;
}

void __cdecl operator delete(
    PVOID pVoid,
    POOL_FLAGS /*poolFlags*/,
    ULONG /*tag*/)
{
    if (pVoid)
    {
        ExFreePool(pVoid);
    }
}

void __cdecl operator delete(
    PVOID pVoid,
    size_t /*cbSize*/)
{
    if (pVoid)
    {
        ExFreePool(pVoid);
    }
}
