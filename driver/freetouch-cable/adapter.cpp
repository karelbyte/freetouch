/*++

FreeTouch virtual audio cable: driver entry + device setup (modern PortCls).

--*/

#define INITGUID
#include "adapter.h"
#include "NewDelete.h"
#include "topo.h"
#include "wave.h"

#pragma code_seg("PAGE")

FtCableContext::~FtCableContext()
{
    PAGED_CODE();
    if (m_pBuffer)
    {
        ExFreePoolWithTag(m_pBuffer, 'CTcF');
        m_pBuffer = NULL;
    }
}

STDMETHODIMP_(NTSTATUS)
FtCableContext::NonDelegatingQueryInterface(
    _In_ REFIID Interface,
    _Out_ PVOID *Object)
{
    PAGED_CODE();
    if (IsEqualGUIDAligned(Interface, IID_IUnknown))
    {
        *Object = static_cast<PVOID>(
            static_cast<INonDelegatingUnknown *>(this));
    }
    else
    {
        *Object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    NonDelegatingAddRef();
    return STATUS_SUCCESS;
}

NTSTATUS FtCableContext::Init(_In_ PDEVICE_OBJECT DeviceObject)
{
    PAGED_CODE();
    m_pDeviceObject = DeviceObject;
    m_ulBufferBytes = FT_CABLE_BUFFER_BYTES;
    m_pBuffer = (PBYTE)ExAllocatePool2(POOL_FLAG_NON_PAGED, m_ulBufferBytes, 'CTcF');
    if (!m_pBuffer)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    RtlZeroMemory(m_pBuffer, m_ulBufferBytes);
    m_lWritePos = 0;
    m_lReadPos = 0;
    return STATUS_SUCCESS;
}

void FtCableContext::RenderWrite(_In_reads_bytes_(ByteCount) PBYTE Data, _In_ ULONG ByteCount)
{
    if (!m_pBuffer || ByteCount == 0)
    {
        return;
    }
    KIRQL oldIrql;
    KeAcquireSpinLock(&m_Lock, &oldIrql);
    LONG writePos = m_lWritePos;
    LONG readPos = m_lReadPos;
    LONG buffered = writePos - readPos;
    if (buffered < 0)
    {
        buffered += (LONG)m_ulBufferBytes;
    }
    if ((ULONG)buffered + ByteCount > m_ulBufferBytes)
    {
        LONG drop = (LONG)((ULONG)buffered + ByteCount - m_ulBufferBytes);
        readPos += drop;
        if (readPos >= (LONG)m_ulBufferBytes)
        {
            readPos -= (LONG)m_ulBufferBytes;
        }
        m_lReadPos = readPos;
    }
    ULONG first = m_ulBufferBytes - (ULONG)writePos;
    if (first > ByteCount)
    {
        first = ByteCount;
    }
    RtlCopyMemory(m_pBuffer + writePos, Data, first);
    if (ByteCount > first)
    {
        RtlCopyMemory(m_pBuffer, Data + first, ByteCount - first);
    }
    writePos += (LONG)ByteCount;
    if (writePos >= (LONG)m_ulBufferBytes)
    {
        writePos -= (LONG)m_ulBufferBytes;
    }
    m_lWritePos = writePos;
    KeReleaseSpinLock(&m_Lock, oldIrql);
}

ULONG FtCableContext::CaptureRead(_Out_writes_bytes_(ByteCount) PBYTE Data, _In_ ULONG ByteCount)
{
    if (!m_pBuffer || ByteCount == 0)
    {
        return 0;
    }
    KIRQL oldIrql;
    KeAcquireSpinLock(&m_Lock, &oldIrql);
    LONG writePos = m_lWritePos;
    LONG readPos = m_lReadPos;
    LONG buffered = writePos - readPos;
    if (buffered < 0)
    {
        buffered += (LONG)m_ulBufferBytes;
    }
    ULONG toCopy = ByteCount;
    if (toCopy > (ULONG)buffered)
    {
        toCopy = (ULONG)buffered;
    }
    ULONG first = m_ulBufferBytes - (ULONG)readPos;
    if (first > toCopy)
    {
        first = toCopy;
    }
    RtlCopyMemory(Data, m_pBuffer + readPos, first);
    if (toCopy > first)
    {
        RtlCopyMemory(Data + first, m_pBuffer, toCopy - first);
    }
    readPos += (LONG)toCopy;
    if (readPos >= (LONG)m_ulBufferBytes)
    {
        readPos -= (LONG)m_ulBufferBytes;
    }
    m_lReadPos = readPos;
    KeReleaseSpinLock(&m_Lock, oldIrql);
    if (toCopy < ByteCount)
    {
        RtlZeroMemory(Data + toCopy, ByteCount - toCopy);
    }
    return toCopy;
}

// Helper callback functions for CreateFn
static NTSTATUS CreateWave(_Out_ PUNKNOWN *Unknown, _In_ REFCLSID ClassId, _In_opt_ PUNKNOWN UnknownOuter, _In_ POOL_FLAGS PoolFlags, _In_ PUNKNOWN UnknownAdapter, _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(UnknownOuter);
    UNREFERENCED_PARAMETER(PoolFlags);
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return CreateMiniportWaveRTFT(Unknown, ClassId, UnknownOuter, PoolFlags, UnknownAdapter, DeviceContext);
}

static NTSTATUS CreateTopologyRender(_Out_ PUNKNOWN *Unknown, _In_ REFCLSID ClassId, _In_opt_ PUNKNOWN UnknownOuter, _In_ POOL_FLAGS PoolFlags, _In_ PUNKNOWN UnknownAdapter, _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(UnknownOuter);
    UNREFERENCED_PARAMETER(PoolFlags);
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return CreateMiniportTopologyFT_Render(Unknown, ClassId, UnknownOuter, PoolFlags, UnknownAdapter, DeviceContext);
}

static NTSTATUS CreateTopologyCapture(_Out_ PUNKNOWN *Unknown, _In_ REFCLSID ClassId, _In_opt_ PUNKNOWN UnknownOuter, _In_ POOL_FLAGS PoolFlags, _In_ PUNKNOWN UnknownAdapter, _In_opt_ PVOID DeviceContext)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ClassId);
    UNREFERENCED_PARAMETER(UnknownOuter);
    UNREFERENCED_PARAMETER(PoolFlags);
    UNREFERENCED_PARAMETER(UnknownAdapter);
    return CreateMiniportTopologyFT_Capture(Unknown, ClassId, UnknownOuter, PoolFlags, UnknownAdapter, DeviceContext);
}

// Creates port + miniport, inits and registers one subdevice.
static NTSTATUS InstallOneSubdevice(
    _In_ PDEVICE_OBJECT DeviceObject,
    _In_ PIRP Irp,
    _In_ PCWSTR Name,
    _In_ REFCLSID PortClassId,
    _In_ FtCableContext *Context,
    _In_ NTSTATUS (*CreateFn)(PUNKNOWN *, REFCLSID, PUNKNOWN, POOL_FLAGS, PUNKNOWN, PVOID),
    _Out_ PUNKNOWN *OutPort)
{
    PAGED_CODE();
    PPORT port = NULL;
    PUNKNOWN miniport = NULL;
    NTSTATUS ntStatus = PcNewPort(&port, PortClassId);
    if (NT_SUCCESS(ntStatus))
    {
        ntStatus = CreateFn(&miniport, GUID_NULL, NULL, POOL_FLAG_NON_PAGED,
            NULL, Context);
    }
    if (NT_SUCCESS(ntStatus))
    {
        ntStatus = port->Init(DeviceObject, Irp, miniport, NULL, NULL);
    }
    if (NT_SUCCESS(ntStatus))
    {
        ntStatus = PcRegisterSubdevice(DeviceObject, const_cast<PWSTR>(Name), port);
    }
    if (NT_SUCCESS(ntStatus))
    {
        port->AddRef();
        *OutPort = port;
    }
    if (miniport)
    {
        miniport->Release();
    }
    if (port)
    {
        port->Release();
    }
    return ntStatus;
}

extern "C" DRIVER_ADD_DEVICE AddDevice;

NTSTATUS
StartDevice(
    _In_ PDEVICE_OBJECT DeviceObject,
    _In_ PIRP Irp,
    _In_ PRESOURCELIST ResourceList
)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(ResourceList);

    PVOID contextMem = ExAllocatePool2(POOL_FLAG_NON_PAGED,
        sizeof(FtCableContext), 'CTcF');
    if (!contextMem)
    {
        return STATUS_NO_MEMORY; // DEBUG: fallo ExAllocatePool2
    }
    FtCableContext *context = new (contextMem) FtCableContext(NULL);
    if (!context)
    {
        ExFreePoolWithTag(contextMem, 'CTcF');
        return STATUS_NO_MEMORY; // DEBUG: fallo placement new
    }
    context->AddRef();
    NTSTATUS ntStatus = context->Init(DeviceObject);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_NO_MEMORY; // DEBUG: distingue fallo de Init
    }

    // Un endpoint por filtro (forma SysVad): topologia render (1 pin) +
    // wave compartido (4 pines) + topologia captura (1 pin).
    PUNKNOWN topoRenderPort = NULL;
    PUNKNOWN wavePort = NULL;
    PUNKNOWN topoCapturePort = NULL;

    ntStatus = InstallOneSubdevice(DeviceObject, Irp, L"TopologyRender",
        CLSID_PortTopology, context,
        CreateTopologyRender,
        &topoRenderPort);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_DEVICE_NOT_READY; // DEBUG: fallo en topo render
    }

    ntStatus = InstallOneSubdevice(DeviceObject, Irp, L"Wave",
        CLSID_PortWaveRT, context,
        CreateWave,
        &wavePort);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_DEVICE_BUSY; // DEBUG: fallo en wave
    }

    ntStatus = InstallOneSubdevice(DeviceObject, Irp, L"TopologyCapture",
        CLSID_PortTopology, context,
        CreateTopologyCapture,
        &topoCapturePort);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_DEVICE_NOT_READY; // DEBUG: fallo en topo captura
    }

    // Render: wave pin 0 (OUT) -> topo render pin 0 (IN).
    ntStatus = PcRegisterPhysicalConnection(
        DeviceObject, wavePort, 0, topoRenderPort, 0);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_INVALID_DEVICE_REQUEST; // DEBUG: fallo conexion 1
    }
    // Captura: topo captura pin 1 (OUT, lado wave) -> wave pin 2 (IN).
    ntStatus = PcRegisterPhysicalConnection(
        DeviceObject, topoCapturePort, 1, wavePort, 2);
    if (!NT_SUCCESS(ntStatus))
    {
        return STATUS_DEVICE_NOT_CONNECTED; // DEBUG: fallo conexion 2
    }

    if (topoRenderPort)
    {
        topoRenderPort->Release();
    }
    if (wavePort)
    {
        wavePort->Release();
    }
    if (topoCapturePort)
    {
        topoCapturePort->Release();
    }
    context->Release();
    return ntStatus;
}

EXTERN_C NTSTATUS
DriverEntry(
    _In_ PDRIVER_OBJECT DriverObject,
    _In_ PUNICODE_STRING RegistryPathName
)
{
    PAGED_CODE();
    NTSTATUS st = PcInitializeAdapterDriver(DriverObject, RegistryPathName, (PDRIVER_ADD_DEVICE)AddDevice);
    return st;
}
NTSTATUS
AddDevice(
    _In_ PDRIVER_OBJECT DriverObject,
    _In_ PDEVICE_OBJECT PhysicalDeviceObject
)
{
    PAGED_CODE();
    return PcAddAdapterDevice(
        DriverObject,
        PhysicalDeviceObject,
        PCPFNSTARTDEVICE(StartDevice),
        4, // topo + wave por cable, con margen
        0);
}