/*++ FreeTouch virtual cable: topology tables (forma SysVad).

   Render:  pin 0 = lado wave (IN, KSCATEGORY_AUDIO),
            pin 1 = endpoint (OUT, KSNODETYPE_SPEAKER, con jack).
   Captura: pin 0 = endpoint (IN, KSNODETYPE_MICROPHONE, con jack),
            pin 1 = lado wave (OUT, KSCATEGORY_AUDIO).
   Sin nodos (igual que el speaker de SysVad): conexion pin->pin.
--*/
#pragma once

#include <ksmedia.h>

NTSTATUS PropertyHandler_TopologyFilter(_In_ PPCPROPERTY_REQUEST PropertyRequest);

// Rango puente analogico (igual que SysVad).
static KSDATARANGE TopologyPinDataRangesBridge[] =
{
    {
        sizeof(KSDATARANGE),
        0,
        0,
        0,
        STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO),
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_ANALOG),
        STATICGUIDOF(KSDATAFORMAT_SPECIFIER_NONE)
    }
};

static PKSDATARANGE TopologyPinDataRangePointersBridge[] =
{
    &TopologyPinDataRangesBridge[0]
};

static PCPIN_DESCRIPTOR TopologyRenderPins[] =
{
    {
        0, 0, 0, NULL,
        {
            0, NULL, 0, NULL,
            SIZEOF_ARRAY(TopologyPinDataRangePointersBridge),
            TopologyPinDataRangePointersBridge,
            KSPIN_DATAFLOW_IN,
            KSPIN_COMMUNICATION_NONE,
            &KSCATEGORY_AUDIO,
            NULL,
            0
        }
    },
    {
        0, 0, 0, NULL,
        {
            0, NULL, 0, NULL,
            SIZEOF_ARRAY(TopologyPinDataRangePointersBridge),
            TopologyPinDataRangePointersBridge,
            KSPIN_DATAFLOW_OUT,
            KSPIN_COMMUNICATION_NONE,
            &KSNODETYPE_SPEAKER,
            NULL,
            0
        }
    }
};

static PCPIN_DESCRIPTOR TopologyCapturePins[] =
{
    {
        0, 0, 0, NULL,
        {
            0, NULL, 0, NULL,
            SIZEOF_ARRAY(TopologyPinDataRangePointersBridge),
            TopologyPinDataRangePointersBridge,
            KSPIN_DATAFLOW_IN,
            KSPIN_COMMUNICATION_NONE,
            &KSNODETYPE_MICROPHONE,
            NULL,
            0
        }
    },
    {
        0, 0, 0, NULL,
        {
            0, NULL, 0, NULL,
            SIZEOF_ARRAY(TopologyPinDataRangePointersBridge),
            TopologyPinDataRangePointersBridge,
            KSPIN_DATAFLOW_OUT,
            KSPIN_COMMUNICATION_NONE,
            &KSCATEGORY_AUDIO,
            NULL,
            0
        }
    }
};

static PCCONNECTION_DESCRIPTOR TopologyRenderConnections[] =
{
    { PCFILTER_NODE, 0, PCFILTER_NODE, 1 },
};

static PCCONNECTION_DESCRIPTOR TopologyCaptureConnections[] =
{
    { PCFILTER_NODE, 0, PCFILTER_NODE, 1 },
};

// Solo el pin endpoint lleva descripcion de jack (NULL = lado wave).
static KSJACK_DESCRIPTION TopologyRenderJackDesc =
{
    KSAUDIO_SPEAKER_STEREO,
    0xB3C98C,
    eConnTypeUnknown,
    eGeoLocFront,
    eGenLocPrimaryBox,
    ePortConnIntegratedDevice,
    TRUE
};

static PKSJACK_DESCRIPTION TopologyRenderJackDescs[] =
{
    NULL,
    &TopologyRenderJackDesc
};

static KSJACK_DESCRIPTION TopologyCaptureJackDesc =
{
    KSAUDIO_SPEAKER_MONO,
    0x000000,
    eConnTypeUnknown,
    eGeoLocFront,
    eGenLocPrimaryBox,
    ePortConnIntegratedDevice,
    TRUE
};

static PKSJACK_DESCRIPTION TopologyCaptureJackDescs[] =
{
    &TopologyCaptureJackDesc,
    NULL
};

static PCPROPERTY_ITEM TopologyPropertiesTopoFilter[] =
{
    {
        &KSPROPSETID_Jack,
        KSPROPERTY_JACK_DESCRIPTION,
        KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_BASICSUPPORT,
        PropertyHandler_TopologyFilter
    },
    {
        &KSPROPSETID_Jack,
        KSPROPERTY_JACK_DESCRIPTION2,
        KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_BASICSUPPORT,
        PropertyHandler_TopologyFilter
    },
    {
        &KSPROPSETID_Jack,
        KSPROPERTY_JACK_DESCRIPTION3,
        KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_BASICSUPPORT,
        PropertyHandler_TopologyFilter
    }
};

DEFINE_PCAUTOMATION_TABLE_PROP(AutomationTopologyFilter, TopologyPropertiesTopoFilter);

static PCFILTER_DESCRIPTOR TopologyRenderFilterDescriptor =
{
    0,
    &AutomationTopologyFilter,
    sizeof(PCPIN_DESCRIPTOR),
    SIZEOF_ARRAY(TopologyRenderPins),
    TopologyRenderPins,
    0, 0, NULL,
    SIZEOF_ARRAY(TopologyRenderConnections),
    TopologyRenderConnections,
    0, NULL
};

static PCFILTER_DESCRIPTOR TopologyCaptureFilterDescriptor =
{
    0,
    &AutomationTopologyFilter,
    sizeof(PCPIN_DESCRIPTOR),
    SIZEOF_ARRAY(TopologyCapturePins),
    TopologyCapturePins,
    0, 0, NULL,
    SIZEOF_ARRAY(TopologyCaptureConnections),
    TopologyCaptureConnections,
    0, NULL
};
