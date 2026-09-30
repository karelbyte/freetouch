/*++ FreeTouch virtual cable: WaveRT miniport + streams. --*/
#pragma once

#include <portcls.h>
#include <stdunk.h>
#include <ks.h>
#include <ksmedia.h>
#include "guids.h"
#include "adapter.h"

NTSTATUS CreateMiniportWaveRTFT(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

NTSTATUS CreateMiniportWaveRTFT_Render(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

NTSTATUS CreateMiniportWaveRTFT_Capture(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

class CMiniportWaveRTFT : public IMiniportWaveRT, public CUnknown
{
public:
    class FtCableContext *m_pCable;
    PPORTWAVERT           m_pPort;
    BOOLEAN               m_bRender;

    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CMiniportWaveRTFT);
    ~CMiniportWaveRTFT();

    FtCableContext *GetCable(void) { return m_pCable; }

    IMP_IMiniportWaveRT;
};

class CMiniportWaveRTStreamFT : public IMiniportWaveRTStreamNotification, public CUnknown
{
private:
    CMiniportWaveRTFT  *m_pMiniport;
    class FtCableContext *m_pCable;
    BOOLEAN             m_bCapture;
    BOOLEAN             m_bRunning;
    ULONG               m_ulSampleRate;
    USHORT              m_usChannels;
    USHORT              m_usBitsPerSample;
    ULONG               m_ulByteRate;
    PBYTE               m_pBuffer;
    PBYTE               m_pAllocBuffer;
    ULONG               m_ulBufferBytes;
    PMDL                m_pMdl;
    BOOLEAN             m_bMdlLocked;
    volatile LONG       m_lPosition;
    LONG                m_lPushed;
    ULONGLONG           m_itBase;
    LONG                m_lBasePos;
    PKEVENT             m_pNotifyEvent;
    KTIMER              m_Timer;
    KDPC                m_Dpc;
    BOOLEAN             m_bTimerOn;
    KSPIN_LOCK          m_Lock;

    void Pump(void);
    static VOID DpcRoutine(_In_ PKDPC Dpc, _In_opt_ PVOID Context, _In_opt_ PVOID Arg1, _In_opt_ PVOID Arg2);
    NTSTATUS FtAllocCommon(
        _Out_ PMDL *AudioBufferMdl,
        _Out_ ULONG *ActualSize,
        _Out_ ULONG *OffsetFromFirstPage,
        _Out_ MEMORY_CACHING_TYPE *CacheType);
    VOID FtFreeCommon(void);

public:
    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CMiniportWaveRTStreamFT);
    ~CMiniportWaveRTStreamFT();

    NTSTATUS Init(
        _In_ CMiniportWaveRTFT *Miniport,
        _In_ ULONG PinId,
        _In_ BOOLEAN Capture,
        _In_ PKSDATAFORMAT DataFormat);

    IMP_IMiniportWaveRTStream;
    IMP_IMiniportWaveRTStreamNotification;
};
