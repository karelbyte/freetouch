/*++ FreeTouch virtual cable: WaveRT miniport + streams.

Render stream: audio engine writes into the cyclic buffer; a DPC pump
copies fresh data into the adapter ring shared with the capture side.
Capture stream: DPC copies from the ring into the cyclic buffer.

--*/
#include "wave.h"
#include "wavtable.h"
#include "adapter.h"
#include "NewDelete.h"

#pragma code_seg("PAGE")

// ---------------------------------------------------------------------
// CMiniportWaveRTFT
// ---------------------------------------------------------------------
CMiniportWaveRTFT::~CMiniportWaveRTFT()
{
    PAGED_CODE();
    if (m_pCable)
    {
        m_pCable->Release();
        m_pCable = NULL;
    }
}

static NTSTATUS CreateMiniportWaveRTFT_Common(
    _Out_ PUNKNOWN *Unknown,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_opt_ PVOID DeviceContext,
    _In_ BOOLEAN Render);

NTSTATUS CreateMiniportWaveRTFT(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(UnknownAdapter);

    UNREFERENCED_PARAMETER(PoolFlags);
    return CreateMiniportWaveRTFT_Common(Unknown, UnknownOuter, DeviceContext, TRUE);
}

static NTSTATUS CreateMiniportWaveRTFT_Common(
    _Out_ PUNKNOWN *Unknown,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_opt_ PVOID DeviceContext,
    _In_ BOOLEAN Render)
{
    PAGED_CODE();
    PVOID objMem = ExAllocatePool2(POOL_FLAG_NON_PAGED,
        sizeof(CMiniportWaveRTFT), 'vWpF');
    if (!objMem)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    CMiniportWaveRTFT *obj = new (objMem) CMiniportWaveRTFT(UnknownOuter);
    obj->AddRef();
    obj->m_pCable = static_cast<FtCableContext *>(DeviceContext);
    if (obj->m_pCable)
    {
        obj->m_pCable->AddRef();
    }
    obj->m_bRender = Render;
    *Unknown = static_cast<IMiniportWaveRT *>(obj);
    return STATUS_SUCCESS;
}

NTSTATUS CreateMiniportWaveRTFT_Render(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(PoolFlags);
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return CreateMiniportWaveRTFT_Common(Unknown, UnknownOuter, DeviceContext, TRUE);
}

NTSTATUS CreateMiniportWaveRTFT_Capture(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(PoolFlags);
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return CreateMiniportWaveRTFT_Common(Unknown, UnknownOuter, DeviceContext, FALSE);
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::GetDescription(_Out_ PPCFILTER_DESCRIPTOR *OutFilterDescriptor)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(m_bRender);
    *OutFilterDescriptor = &FtWaveMiniportFilterDescriptor;
    return STATUS_SUCCESS;
}

// Acepta el formato propuesto para los pines streaming (1 y 3).
// Sin esto, el motor de audio no abre ningun pin del filtro wave.
NTSTATUS PropertyHandler_WaveFilter(_In_ PPCPROPERTY_REQUEST PropertyRequest)
{
    PAGED_CODE();
    NTSTATUS ntStatus = STATUS_INVALID_DEVICE_REQUEST;

    ULONG propId = PropertyRequest->PropertyItem
        ? PropertyRequest->PropertyItem->Id
        : ULONG(-1);
    if (propId != ULONG(KSPROPERTY_PIN_PROPOSEDATAFORMAT))
    {
        return ntStatus;
    }

    if (PropertyRequest->Verb & KSPROPERTY_TYPE_BASICSUPPORT)
    {
        if (PropertyRequest->ValueSize >= sizeof(ULONG))
        {
            *(PULONG)PropertyRequest->Value =
                KSPROPERTY_TYPE_SET | KSPROPERTY_TYPE_BASICSUPPORT;
            PropertyRequest->ValueSize = sizeof(ULONG);
            ntStatus = STATUS_SUCCESS;
        }
        else if (PropertyRequest->ValueSize == 0)
        {
            PropertyRequest->ValueSize = sizeof(ULONG);
            ntStatus = STATUS_BUFFER_OVERFLOW;
        }
        else
        {
            ntStatus = STATUS_BUFFER_TOO_SMALL;
        }
        return ntStatus;
    }

    if (!(PropertyRequest->Verb & KSPROPERTY_TYPE_SET))
    {
        return ntStatus;
    }

    ULONG pinId = ULONG(-1);
    if (PropertyRequest->InstanceSize >= sizeof(ULONG))
    {
        pinId = *(PULONG(PropertyRequest->Instance));
    }
    if (pinId != KSPIN_WAVE_RENDER_SINK && pinId != KSPIN_WAVE_CAPTURE_SOURCE)
    {
        return STATUS_INVALID_PARAMETER;
    }
    if (PropertyRequest->ValueSize < sizeof(KSDATAFORMAT_WAVEFORMATEX))
    {
        return STATUS_BUFFER_TOO_SMALL;
    }
    PKSDATAFORMAT_WAVEFORMATEX fmt =
        reinterpret_cast<PKSDATAFORMAT_WAVEFORMATEX>(PropertyRequest->Value);
    if (fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_PCM &&
        fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_IEEE_FLOAT &&
        fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_EXTENSIBLE)
    {
        return STATUS_INVALID_PARAMETER;
    }
    // Unico formato del cable: estereo 48kHz 16-bit (ambos extremos).
    if (fmt->WaveFormatEx.nSamplesPerSec != 48000 ||
        fmt->WaveFormatEx.nChannels != 2 ||
        fmt->WaveFormatEx.wBitsPerSample != 16)
    {
        return STATUS_NO_MATCH;
    }
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::DataRangeIntersection(
    _In_ ULONG PinId,
    _In_ PKSDATARANGE ClientDataRange,
    _In_ PKSDATARANGE MyDataRange,
    _In_ ULONG OutputBufferLength,
    _Out_writes_bytes_to_opt_(OutputBufferLength, *ResultantFormatLength)
        PVOID ResultantFormat,
    _Out_ PULONG ResultantFormatLength)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(PinId);

    if (!ClientDataRange || !MyDataRange || !ResultantFormatLength)
    {
        return STATUS_INVALID_PARAMETER;
    }

    ULONG required = sizeof(KSDATAFORMAT_WAVEFORMATEXTENSIBLE);
    *ResultantFormatLength = required;
    if (OutputBufferLength < required)
    {
        return STATUS_BUFFER_OVERFLOW;
    }
    if (ClientDataRange->FormatSize < sizeof(KSDATAFORMAT_WAVEFORMATEX) ||
        MyDataRange->FormatSize < sizeof(KSDATAFORMAT_WAVEFORMATEX))
    {
        return STATUS_NO_MATCH;
    }

    PKSDATAFORMAT_WAVEFORMATEX client =
        reinterpret_cast<PKSDATAFORMAT_WAVEFORMATEX>(ClientDataRange);
    PKSDATAFORMAT_WAVEFORMATEX mine =
        reinterpret_cast<PKSDATAFORMAT_WAVEFORMATEX>(MyDataRange);

    if (client->WaveFormatEx.nChannels != mine->WaveFormatEx.nChannels ||
        client->WaveFormatEx.nSamplesPerSec != mine->WaveFormatEx.nSamplesPerSec ||
        client->WaveFormatEx.wBitsPerSample != mine->WaveFormatEx.wBitsPerSample)
    {
        return STATUS_NO_MATCH;
    }

    RtlCopyMemory(ResultantFormat, mine, required);
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::NewStream(
    _Out_ PMINIPORTWAVERTSTREAM *Stream,
    _In_ PPORTWAVERTSTREAM PortStream,
    _In_ ULONG Pin,
    _In_ BOOLEAN Capture,
    _In_ PKSDATAFORMAT DataFormat)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(PortStream);

    // Pin 1 = streaming render, pin 3 = streaming captura.
    if ((Pin != KSPIN_WAVE_RENDER_SINK && Pin != KSPIN_WAVE_CAPTURE_SOURCE) || !Stream)
    {
        return STATUS_INVALID_PARAMETER;
    }
    BOOLEAN wantCapture = (Pin == KSPIN_WAVE_CAPTURE_SOURCE) ? TRUE : Capture;

    PVOID streamMem = ExAllocatePool2(POOL_FLAG_NON_PAGED,
        sizeof(CMiniportWaveRTStreamFT), 'sWpF');
    if (!streamMem)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    CMiniportWaveRTStreamFT *obj = new (streamMem) CMiniportWaveRTStreamFT(NULL);
    if (!obj)
    {
        ExFreePoolWithTag(streamMem, 'sWpF');
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    obj->AddRef();
    NTSTATUS ntStatus = obj->Init(this, Pin, wantCapture, DataFormat);
    if (!NT_SUCCESS(ntStatus))
    {
        obj->Release();
        return ntStatus;
    }
    *Stream = static_cast<IMiniportWaveRTStream *>(obj);
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::GetDeviceDescription(_Out_ PDEVICE_DESCRIPTION DeviceDescription)
{
    PAGED_CODE();
    if (!DeviceDescription)
    {
        return STATUS_INVALID_PARAMETER;
    }
    RtlZeroMemory(DeviceDescription, sizeof(DEVICE_DESCRIPTION));
    DeviceDescription->Master = TRUE;
    DeviceDescription->ScatterGather = TRUE;
    DeviceDescription->Dma32BitAddresses = TRUE;
    DeviceDescription->InterfaceType = PCIBus;
    DeviceDescription->MaximumLength = 0xFFFFFFFF;
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::Init(
    _In_ PUNKNOWN UnknownAdapter,
    _In_ PRESOURCELIST ResourceList,
    _In_ PPORTWAVERT Port)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ResourceList);
    m_pPort = Port;
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return STATUS_SUCCESS;
}

#pragma code_seg()

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTFT::NonDelegatingQueryInterface(
    _In_ REFIID Interface,
    _Out_ PVOID *Object)
{
    PAGED_CODE();
    if (IsEqualGUIDAligned(Interface, IID_IUnknown))
    {
        *Object = PVOID(PUNKNOWN(this));
    }
    else if (IsEqualGUIDAligned(Interface, IID_IMiniport))
    {
        *Object = PVOID(PMINIPORT(this));
    }
    else if (IsEqualGUIDAligned(Interface, IID_IMiniportWaveRT))
    {
        *Object = PVOID(PMINIPORTWAVERT(this));
    }
    else
    {
        *Object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    reinterpret_cast<PUNKNOWN>(*Object)->AddRef();
    return STATUS_SUCCESS;
}

// ---------------------------------------------------------------------
// CMiniportWaveRTStreamFT
// ---------------------------------------------------------------------
STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::NonDelegatingQueryInterface(
    _In_ REFIID Interface,
    _Out_ PVOID *Object)
{
    if (IsEqualGUIDAligned(Interface, IID_IUnknown))
    {
        *Object = PVOID(PUNKNOWN(this));
    }
    else if (IsEqualGUIDAligned(Interface, IID_IMiniportWaveRTStream))
    {
        *Object = PVOID(PMINIPORTWAVERTSTREAM(this));
    }
    else if (IsEqualGUIDAligned(Interface, IID_IMiniportWaveRTStreamNotification))
    {
        *Object = PVOID(PMINIPORTWAVERTSTREAM(this));
    }
    else
    {
        *Object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    reinterpret_cast<PUNKNOWN>(*Object)->AddRef();
    return STATUS_SUCCESS;
}

CMiniportWaveRTStreamFT::~CMiniportWaveRTStreamFT()
{
    if (m_bTimerOn)
    {
        KeCancelTimer(&m_Timer);
        KeRemoveQueueDpc(&m_Dpc);
        m_bTimerOn = FALSE;
    }
    if (m_pBuffer && m_pMdl)
    {
        MmUnmapLockedPages(m_pBuffer, m_pMdl);
        m_pBuffer = NULL;
    }
    if (m_pMdl)
    {
        if (m_bMdlLocked)
        {
            MmUnlockPages(m_pMdl);
            m_bMdlLocked = FALSE;
        }
        IoFreeMdl(m_pMdl);
        m_pMdl = NULL;
    }
    if (m_pAllocBuffer)
    {
        ExFreePoolWithTag(m_pAllocBuffer, 'bWpF');
        m_pAllocBuffer = NULL;
    }
    if (m_pMiniport)
    {
        m_pMiniport->Release();
        m_pMiniport = NULL;
    }
    if (m_pCable)
    {
        m_pCable->Release();
        m_pCable = NULL;
    }
}

NTSTATUS CMiniportWaveRTStreamFT::Init(
    _In_ CMiniportWaveRTFT *Miniport,
    _In_ ULONG PinId,
    _In_ BOOLEAN Capture,
    _In_ PKSDATAFORMAT DataFormat)
{
    UNREFERENCED_PARAMETER(PinId);
    m_pMiniport = Miniport;
    m_pMiniport->AddRef();
    m_pCable = Miniport->GetCable();
    if (m_pCable)
    {
        m_pCable->AddRef();
    }
    m_bCapture = Capture;
    m_itBase = 0;
    m_lBasePos = 0;
    m_pNotifyEvent = NULL;
    KeInitializeSpinLock(&m_Lock);
    KeInitializeTimer(&m_Timer);
    KeInitializeDpc(&m_Dpc, DpcRoutine, this);
    return SetFormat(DataFormat);
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::SetFormat(_In_ PKSDATAFORMAT DataFormat)
{
    if (DataFormat->FormatSize < sizeof(KSDATAFORMAT_WAVEFORMATEX))
    {
        return STATUS_INVALID_PARAMETER;
    }
    PKSDATAFORMAT_WAVEFORMATEX fmt =
        reinterpret_cast<PKSDATAFORMAT_WAVEFORMATEX>(DataFormat);
    if (fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_PCM &&
        fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_IEEE_FLOAT &&
        fmt->WaveFormatEx.wFormatTag != WAVE_FORMAT_EXTENSIBLE)
    {
        return STATUS_INVALID_PARAMETER;
    }
    // Unico formato del cable: estereo 48kHz 16-bit (ambos extremos).
    if (fmt->WaveFormatEx.nSamplesPerSec != 48000 ||
        fmt->WaveFormatEx.nChannels != 2 ||
        fmt->WaveFormatEx.wBitsPerSample != 16)
    {
        return STATUS_INVALID_PARAMETER;
    }
    m_ulSampleRate = fmt->WaveFormatEx.nSamplesPerSec;
    m_usChannels = fmt->WaveFormatEx.nChannels;
    m_usBitsPerSample = fmt->WaveFormatEx.wBitsPerSample;
    m_ulByteRate = m_ulSampleRate * m_usChannels * (m_usBitsPerSample / 8);
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::SetState(_In_ KSSTATE State)
{
    if (State == KSSTATE_RUN)
    {
        if (!m_bRunning)
        {
            // Re-ancla el reloj a la posicion actual (continuo, sin saltos).
            m_itBase = KeQueryInterruptTime();
            m_lBasePos = m_lPosition;
        }
        if (!m_bTimerOn)
        {
            LARGE_INTEGER due;
            due.QuadPart = -10 * 1000 * 5; // 5 ms
            KeSetTimerEx(&m_Timer, due, 5, &m_Dpc);
            m_bTimerOn = TRUE;
        }
        m_bRunning = TRUE;
    }
    else
    {
        m_bRunning = FALSE;
        if (State == KSSTATE_STOP && m_bTimerOn)
        {
            KeCancelTimer(&m_Timer);
            KeRemoveQueueDpc(&m_Dpc);
            m_bTimerOn = FALSE;
        }
    }
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::GetPosition(_Out_ PKSAUDIO_POSITION Position)
{
    if (!Position || m_ulBufferBytes == 0)
    {
        return STATUS_INVALID_PARAMETER;
    }
    Position->PlayOffset = (ULONG)(m_lPosition % (LONG)m_ulBufferBytes);
    Position->WriteOffset = Position->PlayOffset;
    return STATUS_SUCCESS;
}

NTSTATUS CMiniportWaveRTStreamFT::FtAllocCommon(
    _Out_ PMDL *AudioBufferMdl,
    _Out_ ULONG *ActualSize,
    _Out_ ULONG *OffsetFromFirstPage,
    _Out_ MEMORY_CACHING_TYPE *CacheType)
{
    ULONG size = 48000 * 2 * 4 * 2; // 2s stereo float
    size = (size + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);
    PBYTE buffer = (PBYTE)ExAllocatePool2(
        POOL_FLAG_NON_PAGED | POOL_FLAG_CACHE_ALIGNED, size, 'bWpF');
    if (!buffer)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    RtlZeroMemory(buffer, size);
    PMDL mdl = IoAllocateMdl(buffer, size, FALSE, FALSE, NULL);
    if (!mdl)
    {
        ExFreePoolWithTag(buffer, 'bWpF');
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    __try
    {
        MmProbeAndLockPages(mdl, KernelMode, IoModifyAccess);
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        IoFreeMdl(mdl);
        ExFreePoolWithTag(buffer, 'bWpF');
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    m_pBuffer = (PBYTE)MmMapLockedPagesSpecifyCache(
        mdl, KernelMode, MmNonCached, NULL, FALSE, NormalPagePriority);
    if (!m_pBuffer)
    {
        MmUnlockPages(mdl);
        IoFreeMdl(mdl);
        ExFreePoolWithTag(buffer, 'bWpF');
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    m_pMdl = mdl;
    m_bMdlLocked = TRUE;
    m_pAllocBuffer = buffer;
    m_ulBufferBytes = size;
    m_lPosition = 0;
    m_lPushed = 0;
    *AudioBufferMdl = mdl;
    *ActualSize = size;
    *OffsetFromFirstPage = 0;
    *CacheType = MmNonCached;
    return STATUS_SUCCESS;
}

VOID CMiniportWaveRTStreamFT::FtFreeCommon(void)
{
    if (m_pBuffer && m_pMdl)
    {
        MmUnmapLockedPages(m_pBuffer, m_pMdl);
        m_pBuffer = NULL;
    }
    if (m_pMdl)
    {
        if (m_bMdlLocked)
        {
            MmUnlockPages(m_pMdl);
            m_bMdlLocked = FALSE;
        }
        IoFreeMdl(m_pMdl);
        m_pMdl = NULL;
    }
    if (m_pAllocBuffer)
    {
        ExFreePoolWithTag(m_pAllocBuffer, 'bWpF');
        m_pAllocBuffer = NULL;
    }
    m_ulBufferBytes = 0;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::AllocateAudioBuffer(
    _In_ ULONG RequestedSize,
    _Out_ PMDL *AudioBufferMdl,
    _Out_ ULONG *ActualSize,
    _Out_ ULONG *OffsetFromFirstPage,
    _Out_ MEMORY_CACHING_TYPE *CacheType)
{
    UNREFERENCED_PARAMETER(RequestedSize);
    FtFreeCommon();
    return FtAllocCommon(AudioBufferMdl, ActualSize, OffsetFromFirstPage, CacheType);
}

STDMETHODIMP_(VOID)
CMiniportWaveRTStreamFT::FreeAudioBuffer(
    _In_opt_ PMDL AudioBufferMdl,
    _In_ ULONG BufferSize)
{
    UNREFERENCED_PARAMETER(AudioBufferMdl);
    UNREFERENCED_PARAMETER(BufferSize);
    FtFreeCommon();
}

STDMETHODIMP_(VOID)
CMiniportWaveRTStreamFT::GetHWLatency(_Out_ KSRTAUDIO_HWLATENCY *hwLatency)
{
    if (hwLatency)
    {
        hwLatency->FifoSize = 0;
        hwLatency->ChipsetDelay = 0;
        hwLatency->CodecDelay = 0;
    }
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::GetPositionRegister(_Out_ KSRTAUDIO_HWREGISTER *Register)
{
    UNREFERENCED_PARAMETER(Register);
    return STATUS_NOT_SUPPORTED;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::GetClockRegister(_Out_ KSRTAUDIO_HWREGISTER *Register)
{
    UNREFERENCED_PARAMETER(Register);
    return STATUS_NOT_SUPPORTED;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::AllocateBufferWithNotification(
    _In_ ULONG NotificationCount,
    _In_ ULONG RequestedSize,
    _Out_ PMDL *AudioBufferMdl,
    _Out_ ULONG *ActualSize,
    _Out_ ULONG *OffsetFromFirstPage,
    _Out_ MEMORY_CACHING_TYPE *CacheType)
{
    UNREFERENCED_PARAMETER(NotificationCount);
    UNREFERENCED_PARAMETER(RequestedSize);
    FtFreeCommon();
    return FtAllocCommon(AudioBufferMdl, ActualSize, OffsetFromFirstPage, CacheType);
}

STDMETHODIMP_(VOID)
CMiniportWaveRTStreamFT::FreeBufferWithNotification(
    _In_ PMDL AudioBufferMdl,
    _In_ ULONG BufferSize)
{
    UNREFERENCED_PARAMETER(AudioBufferMdl);
    UNREFERENCED_PARAMETER(BufferSize);
    FtFreeCommon();
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::RegisterNotificationEvent(_In_ PKEVENT NotificationEvent)
{
    m_pNotifyEvent = NotificationEvent;
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportWaveRTStreamFT::UnregisterNotificationEvent(_In_ PKEVENT NotificationEvent)
{
    UNREFERENCED_PARAMETER(NotificationEvent);
    m_pNotifyEvent = NULL;
    return STATUS_SUCCESS;
}

VOID CMiniportWaveRTStreamFT::DpcRoutine(
    _In_ PKDPC /*Dpc*/,
    _In_opt_ PVOID Context,
    _In_opt_ PVOID /*Arg1*/,
    _In_opt_ PVOID /*Arg2*/)
{
    CMiniportWaveRTStreamFT *that =
        reinterpret_cast<CMiniportWaveRTStreamFT *>(Context);
    if (that)
    {
        that->Pump();
    }
}

void CMiniportWaveRTStreamFT::Pump(void)
{
    if (!m_bRunning || !m_pBuffer || m_ulBufferBytes == 0 || m_ulByteRate == 0)
    {
        return;
    }
    // Posicion anclada al reloj real (unidades de 100ns): avanza lo
    // transcurrido de verdad, no un paso fijo por tick (los DPC tienen
    // jitter y el retardo se acumulaba -> el audio (y el video atado
    // a el) iba lento).
    ULONG chunk = 1;
    {
        ULONGLONG now = KeQueryInterruptTime();
        ULONGLONG elapsed = (now >= m_itBase) ? (now - m_itBase) : 0;
        LONGLONG expected = (LONGLONG)m_lBasePos +
            (LONGLONG)((elapsed * (ULONGLONG)m_ulByteRate) / 10000000ULL);
        LONGLONG delta = expected - (LONGLONG)m_lPosition;
        LONGLONG maxChunk = ((LONGLONG)m_ulByteRate * 50) / 1000; // tope 50ms
        if (delta < 0)
        {
            delta = 0;
        }
        if (delta > maxChunk)
        {
            delta = maxChunk;
        }
        if (delta > 0)
        {
            chunk = (ULONG)delta;
        }
    }
    if (!m_pCable)
    {
        return;
    }
    if (!m_bCapture)
    {
        ULONG bufBytes = m_ulBufferBytes;
        LONG pos = m_lPushed % (LONG)bufBytes;
        ULONG first = bufBytes - (ULONG)pos;
        if (first > chunk)
        {
            first = chunk;
        }
        m_pCable->RenderWrite(m_pBuffer + pos, first);
        if (chunk > first)
        {
            m_pCable->RenderWrite(m_pBuffer, chunk - first);
        }
        m_lPushed += (LONG)chunk;
        m_lPosition += (LONG)chunk;
    }
    else
    {
        ULONG bufBytes = m_ulBufferBytes;
        LONG pos = m_lPosition % (LONG)bufBytes;
        UCHAR staging[1024];
        ULONG remaining = chunk;
        while (remaining > 0)
        {
            ULONG want = remaining > sizeof(staging) ? sizeof(staging) : remaining;
            ULONG got = m_pCable->CaptureRead(staging, want);
            ULONG first = bufBytes - (ULONG)pos;
            if (first > got)
            {
                first = got;
            }
            if (first > 0)
            {
                RtlCopyMemory(m_pBuffer + pos, staging, first);
                pos += (LONG)first;
                if (pos >= (LONG)bufBytes)
                {
                    pos -= (LONG)bufBytes;
                }
            }
            if (got < want)
            {
                ULONG zfirst = bufBytes - (ULONG)pos;
                ULONG zleft = want - got;
                if (zfirst > zleft)
                {
                    zfirst = zleft;
                }
                if (zfirst > 0)
                {
                    RtlZeroMemory(m_pBuffer + pos, zfirst);
                    pos += (LONG)zfirst;
                    if (pos >= (LONG)bufBytes)
                    {
                        pos -= (LONG)bufBytes;
                    }
                }
                if (zleft > zfirst)
                {
                    RtlZeroMemory(m_pBuffer, zleft - zfirst);
                    pos = (LONG)(zleft - zfirst);
                }
            }
            remaining -= want;
        }
        m_lPosition += (LONG)chunk;
    }
    if (m_pNotifyEvent)
    {
        KeSetEvent(m_pNotifyEvent, IO_NO_INCREMENT, FALSE);
    }
}
