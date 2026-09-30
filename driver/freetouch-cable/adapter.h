/*++

FreeTouch virtual audio cable: per-cable shared context.
One driver instance = one cable (render endpoint + capture endpoint
sharing a ring buffer). The INF installs it twice (Cable A / Cable B).

--*/

#pragma once

#include <portcls.h>
#include <stdunk.h>
#include <ks.h>
#include <ksmedia.h>

// Size of the shared loopback ring (2s stereo 48k float).
#define FT_CABLE_BUFFER_BYTES (48000 * 2 * sizeof(float) * 2)

// ---------------------------------------------------------------------
// FtCableContext: refcounted per-cable shared state.
// ---------------------------------------------------------------------
class FtCableContext : public CUnknown
{
private:
    PDEVICE_OBJECT m_pDeviceObject;
    PBYTE          m_pBuffer;
    ULONG          m_ulBufferBytes;
    volatile LONG  m_lWritePos;    // bytes written by render side
    volatile LONG  m_lReadPos;     // bytes consumed by capture side
    KSPIN_LOCK     m_Lock;

public:
    DECLARE_STD_UNKNOWN();
    FtCableContext(_In_opt_ PUNKNOWN OuterUnknown)
        : CUnknown(OuterUnknown), m_pDeviceObject(NULL),
          m_pBuffer(NULL), m_ulBufferBytes(0),
          m_lWritePos(0), m_lReadPos(0)
    {
        KeInitializeSpinLock(&m_Lock);
    }
    ~FtCableContext();

    NTSTATUS Init(_In_ PDEVICE_OBJECT DeviceObject);

    void RenderWrite(_In_reads_bytes_(ByteCount) PBYTE Data, _In_ ULONG ByteCount);
    ULONG CaptureRead(_Out_writes_bytes_(ByteCount) PBYTE Data, _In_ ULONG ByteCount);
};
