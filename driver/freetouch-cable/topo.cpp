/*++ FreeTouch virtual cable: topology miniport. --*/
#include "topo.h"
#include "NewDelete.h"
#include "toptable.h"
#include "adapter.h"

#pragma code_seg("PAGE")

// Handlers de jack copiando el patron de SysVad (mintopo.cpp):
// el pin se lee de PropertyRequest->Instance; solo el pin endpoint
// tiene descripcion (el otro es NULL).
static NTSTATUS FtJackBasicSupport(_In_ PPCPROPERTY_REQUEST PropertyRequest)
{
    PAGED_CODE();
    if (PropertyRequest->ValueSize >= sizeof(ULONG))
    {
        *(PULONG)PropertyRequest->Value =
            KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_BASICSUPPORT;
        PropertyRequest->ValueSize = sizeof(ULONG);
        return STATUS_SUCCESS;
    }
    if (PropertyRequest->ValueSize == 0)
    {
        PropertyRequest->ValueSize = sizeof(ULONG);
        return STATUS_BUFFER_OVERFLOW;
    }
    return STATUS_BUFFER_TOO_SMALL;
}
static NTSTATUS FtJackDescription(
    _In_ PPCPROPERTY_REQUEST PropertyRequest,
    _In_ ULONG JackCount,
    _In_reads_(JackCount) PKSJACK_DESCRIPTION *JackDescs)
{
    PAGED_CODE();
    NTSTATUS ntStatus = STATUS_INVALID_DEVICE_REQUEST;
    ULONG pinId = ULONG(-1);

    if (PropertyRequest->InstanceSize >= sizeof(ULONG))
    {
        pinId = *(PULONG(PropertyRequest->Instance));
        if (pinId < JackCount && JackDescs[pinId] != NULL)
        {
            if (PropertyRequest->Verb & KSPROPERTY_TYPE_BASICSUPPORT)
            {
                ntStatus = FtJackBasicSupport(PropertyRequest);
            }
            else
            {
                ULONG cbNeeded = sizeof(KSMULTIPLE_ITEM) + sizeof(KSJACK_DESCRIPTION);
                if (PropertyRequest->ValueSize == 0)
                {
                    PropertyRequest->ValueSize = cbNeeded;
                    ntStatus = STATUS_BUFFER_OVERFLOW;
                }
                else if (PropertyRequest->ValueSize < cbNeeded)
                {
                    ntStatus = STATUS_BUFFER_TOO_SMALL;
                }
                else if (PropertyRequest->Verb & KSPROPERTY_TYPE_GET)
                {
                    PKSMULTIPLE_ITEM mi = (PKSMULTIPLE_ITEM)PropertyRequest->Value;
                    PKSJACK_DESCRIPTION desc = (PKSJACK_DESCRIPTION)(mi + 1);
                    mi->Size = cbNeeded;
                    mi->Count = 1;
                    RtlCopyMemory(desc, JackDescs[pinId], sizeof(KSJACK_DESCRIPTION));
                    ntStatus = STATUS_SUCCESS;
                }
            }
        }
    }
    return ntStatus;
}

static NTSTATUS FtJackDescription2(
    _In_ PPCPROPERTY_REQUEST PropertyRequest,
    _In_ ULONG JackCount,
    _In_reads_(JackCount) PKSJACK_DESCRIPTION *JackDescs,
    _In_ DWORD JackCapabilities)
{
    PAGED_CODE();
    NTSTATUS ntStatus = STATUS_INVALID_DEVICE_REQUEST;
    ULONG pinId = ULONG(-1);

    if (PropertyRequest->InstanceSize >= sizeof(ULONG))
    {
        pinId = *(PULONG(PropertyRequest->Instance));
        if (pinId < JackCount && JackDescs[pinId] != NULL)
        {
            if (PropertyRequest->Verb & KSPROPERTY_TYPE_BASICSUPPORT)
            {
                ntStatus = FtJackBasicSupport(PropertyRequest);
            }
            else
            {
                ULONG cbNeeded = sizeof(KSMULTIPLE_ITEM) + sizeof(KSJACK_DESCRIPTION2);
                if (PropertyRequest->ValueSize == 0)
                {
                    PropertyRequest->ValueSize = cbNeeded;
                    ntStatus = STATUS_BUFFER_OVERFLOW;
                }
                else if (PropertyRequest->ValueSize < cbNeeded)
                {
                    ntStatus = STATUS_BUFFER_TOO_SMALL;
                }
                else if (PropertyRequest->Verb & KSPROPERTY_TYPE_GET)
                {
                    PKSMULTIPLE_ITEM mi = (PKSMULTIPLE_ITEM)PropertyRequest->Value;
                    PKSJACK_DESCRIPTION2 desc = (PKSJACK_DESCRIPTION2)(mi + 1);
                    mi->Size = cbNeeded;
                    mi->Count = 1;
                    RtlZeroMemory(desc, sizeof(KSJACK_DESCRIPTION2));
                    desc->DeviceStateInfo = 0;
                    desc->JackCapabilities = JackCapabilities;
                    ntStatus = STATUS_SUCCESS;
                }
            }
        }
    }
    return ntStatus;
}

static NTSTATUS FtJackDescription3(
    _In_ PPCPROPERTY_REQUEST PropertyRequest,
    _In_ ULONG JackCount,
    _In_reads_(JackCount) PKSJACK_DESCRIPTION *JackDescs,
    _In_ ULONG ConfigId)
{
    PAGED_CODE();
    NTSTATUS ntStatus = STATUS_INVALID_DEVICE_REQUEST;
    ULONG pinId = ULONG(-1);

    if (PropertyRequest->InstanceSize >= sizeof(ULONG))
    {
        pinId = *(PULONG(PropertyRequest->Instance));
        if (pinId < JackCount && JackDescs[pinId] != NULL)
        {
            if (PropertyRequest->Verb & KSPROPERTY_TYPE_BASICSUPPORT)
            {
                ntStatus = FtJackBasicSupport(PropertyRequest);
            }
            else
            {
                ULONG cbNeeded = sizeof(KSMULTIPLE_ITEM) + sizeof(KSJACK_DESCRIPTION3);
                if (PropertyRequest->ValueSize == 0)
                {
                    PropertyRequest->ValueSize = cbNeeded;
                    ntStatus = STATUS_BUFFER_OVERFLOW;
                }
                else if (PropertyRequest->ValueSize < cbNeeded)
                {
                    ntStatus = STATUS_BUFFER_TOO_SMALL;
                }
                else if (PropertyRequest->Verb & KSPROPERTY_TYPE_GET)
                {
                    PKSMULTIPLE_ITEM mi = (PKSMULTIPLE_ITEM)PropertyRequest->Value;
                    PKSJACK_DESCRIPTION3 desc = (PKSJACK_DESCRIPTION3)(mi + 1);
                    mi->Size = cbNeeded;
                    mi->Count = 1;
                    RtlZeroMemory(desc, sizeof(KSJACK_DESCRIPTION3));
                    desc->ConfigId = ConfigId;
                    ntStatus = STATUS_SUCCESS;
                }
            }
        }
    }
    return ntStatus;
}

NTSTATUS PropertyHandler_TopologyFilter(_In_ PPCPROPERTY_REQUEST PropertyRequest)
{
    PAGED_CODE();
    CMiniportTopologyFT *miniport =
        static_cast<CMiniportTopologyFT *>(PropertyRequest->MajorTarget);
    if (!miniport)
    {
        return STATUS_INVALID_DEVICE_REQUEST;
    }

    ULONG jackCount;
    PKSJACK_DESCRIPTION *jackDescs;
    if (miniport->m_bRender)
    {
        jackCount = SIZEOF_ARRAY(TopologyRenderJackDescs);
        jackDescs = TopologyRenderJackDescs;
    }
    else
    {
        jackCount = SIZEOF_ARRAY(TopologyCaptureJackDescs);
        jackDescs = TopologyCaptureJackDescs;
    }

    ULONG propId = PropertyRequest->PropertyItem
        ? PropertyRequest->PropertyItem->Id
        : ULONG(-1);
    if (propId == ULONG(KSPROPERTY_JACK_DESCRIPTION))
    {
        return FtJackDescription(PropertyRequest, jackCount, jackDescs);
    }
    if (propId == ULONG(KSPROPERTY_JACK_DESCRIPTION2))
    {
        return FtJackDescription2(PropertyRequest, jackCount, jackDescs, 0);
    }
    if (propId == ULONG(KSPROPERTY_JACK_DESCRIPTION3))
    {
        return FtJackDescription3(PropertyRequest, jackCount, jackDescs, 0);
    }
    return STATUS_INVALID_DEVICE_REQUEST;
}

STDMETHODIMP_(NTSTATUS)
CMiniportTopologyFT::NonDelegatingQueryInterface(
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
    else if (IsEqualGUIDAligned(Interface, IID_IMiniportTopology))
    {
        *Object = PVOID(PMINIPORTTOPOLOGY(this));
    }
    else
    {
        *Object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    reinterpret_cast<PUNKNOWN>(*Object)->AddRef();
    return STATUS_SUCCESS;
}

CMiniportTopologyFT::~CMiniportTopologyFT()
{
    PAGED_CODE();
    if (m_pCable)
    {
        m_pCable->Release();
        m_pCable = NULL;
    }
}

static NTSTATUS CreateMiniportTopologyFT_Common(
    _Out_ PUNKNOWN *Unknown,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_opt_ PVOID DeviceContext,
    _In_ BOOLEAN Render);

NTSTATUS CreateMiniportTopologyFT(
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
    return CreateMiniportTopologyFT_Common(Unknown, UnknownOuter, DeviceContext, TRUE);
}

static NTSTATUS CreateMiniportTopologyFT_Common(
    _Out_ PUNKNOWN *Unknown,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_opt_ PVOID DeviceContext,
    _In_ BOOLEAN Render)
{
    PAGED_CODE();
    PVOID mem = ExAllocatePool2(POOL_FLAG_NON_PAGED,
        sizeof(CMiniportTopologyFT), 'oTpF');
    if (!mem)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    CMiniportTopologyFT *obj = new (mem) CMiniportTopologyFT(UnknownOuter);
    obj->AddRef();
    obj->m_pCable = static_cast<FtCableContext *>(DeviceContext);
    if (obj->m_pCable)
    {
        obj->m_pCable->AddRef();
    }
    obj->m_bRender = Render;
    *Unknown = static_cast<IMiniportTopology *>(obj);
    return STATUS_SUCCESS;
}

NTSTATUS CreateMiniportTopologyFT_Render(
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
    return CreateMiniportTopologyFT_Common(Unknown, UnknownOuter, DeviceContext, TRUE);
}

NTSTATUS CreateMiniportTopologyFT_Capture(
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
    return CreateMiniportTopologyFT_Common(Unknown, UnknownOuter, DeviceContext, FALSE);
}
STDMETHODIMP_(NTSTATUS)
CMiniportTopologyFT::GetDescription(_Out_ PPCFILTER_DESCRIPTOR *OutFilterDescriptor)
{
    PAGED_CODE();
    *OutFilterDescriptor = m_bRender
        ? &TopologyRenderFilterDescriptor
        : &TopologyCaptureFilterDescriptor;
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportTopologyFT::DataRangeIntersection(
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
    UNREFERENCED_PARAMETER(ClientDataRange);
    UNREFERENCED_PARAMETER(MyDataRange);
    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(ResultantFormat);
    UNREFERENCED_PARAMETER(ResultantFormatLength);
    // Return success - the pin data ranges are defined in the descriptor table
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS)
CMiniportTopologyFT::Init(
    _In_ PUNKNOWN UnknownAdapter,
    _In_ PRESOURCELIST ResourceList,
    _In_ PPORTTOPOLOGY Port)
{
    PAGED_CODE();
    UNREFERENCED_PARAMETER(UnknownAdapter);
    UNREFERENCED_PARAMETER(ResourceList);
    UNREFERENCED_PARAMETER(Port);
    return STATUS_SUCCESS;
}
