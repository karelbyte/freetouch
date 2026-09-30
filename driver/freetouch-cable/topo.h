/*++ FreeTouch virtual cable: topology miniport (2 bridge pins, no nodes). --*/
#pragma once

#include <portcls.h>
#include <stdunk.h>
#include "guids.h"

NTSTATUS CreateMiniportTopologyFT(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

NTSTATUS CreateMiniportTopologyFT_Render(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

NTSTATUS CreateMiniportTopologyFT_Capture(
    _Out_ PUNKNOWN *Unknown,
    _In_ REFCLSID ClassId,
    _In_opt_ PUNKNOWN UnknownOuter,
    _In_ POOL_FLAGS PoolFlags,
    _In_ PUNKNOWN UnknownAdapter,
    _In_opt_ PVOID DeviceContext);

class CMiniportTopologyFT : public IMiniportTopology, public CUnknown
{
public:
    class FtCableContext *m_pCable;
    BOOLEAN m_bRender; // TRUE: pin puente IN (render). FALSE: OUT (captura).

    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CMiniportTopologyFT);
    ~CMiniportTopologyFT();

    IMP_IMiniportTopology;
};
