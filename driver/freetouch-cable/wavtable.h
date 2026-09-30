/*++ FreeTouch virtual cable: wave tables (render + capture). --*/
#pragma once

// Unico formato del cable (ambos extremos iguales, sin conversion):
// estereo 48kHz PCM16. Asi los bytes del render sirven tal cual en captura.
static KSDATAFORMAT_WAVEFORMATEXTENSIBLE FtWaveSupportedFormats[] =
{
    { // 0: stereo 48k PCM16
        { sizeof(KSDATAFORMAT_WAVEFORMATEXTENSIBLE), 0, 0, 0,
          STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO),
          STATICGUIDOF(KSDATAFORMAT_SUBTYPE_PCM),
          STATICGUIDOF(KSDATAFORMAT_SPECIFIER_WAVEFORMATEX) },
        { { WAVE_FORMAT_EXTENSIBLE, 2, 48000, 192000, 4, 16,
            sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX) },
          16, KSAUDIO_SPEAKER_STEREO, STATICGUIDOF(KSDATAFORMAT_SUBTYPE_PCM) }
    },

};

static PKSDATARANGE FtWaveDataRangePointers[] =
{
    (PKSDATARANGE)&FtWaveSupportedFormats[0],
};

static KSDATARANGE FtWaveBridgeRange =
{
    sizeof(KSDATARANGE), 0, 0, 0,
    STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO),
    STATICGUIDOF(KSDATAFORMAT_SUBTYPE_ANALOG),
    STATICGUIDOF(KSDATAFORMAT_SPECIFIER_NONE)
};
static PKSDATARANGE FtWaveBridgeRangePtr[] = { &FtWaveBridgeRange };

// Pins: 0 render bridge, 1 render stream, 2 capture bridge, 3 capture stream.
#define KSPIN_WAVE_RENDER_BRIDGE  0
#define KSPIN_WAVE_RENDER_SINK    1
#define KSPIN_WAVE_CAPTURE_BRIDGE 2
#define KSPIN_WAVE_CAPTURE_SOURCE 3

NTSTATUS PropertyHandler_WaveFilter(_In_ PPCPROPERTY_REQUEST PropertyRequest);

static PCPROPERTY_ITEM FtWaveFilterProperties[] =
{
    {
        &KSPROPSETID_Pin,
        KSPROPERTY_PIN_PROPOSEDATAFORMAT,
        KSPROPERTY_TYPE_SET | KSPROPERTY_TYPE_BASICSUPPORT,
        PropertyHandler_WaveFilter
    }
};

DEFINE_PCAUTOMATION_TABLE_PROP(AutomationWaveFilter, FtWaveFilterProperties);

static PCPIN_DESCRIPTOR FtWaveMiniportPins[] =
{
#ifdef FT_WAVE_CAPTURE_ONLY
    { 0, 0, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveBridgeRangePtr), FtWaveBridgeRangePtr,
        KSPIN_DATAFLOW_IN, KSPIN_COMMUNICATION_NONE, &KSCATEGORY_AUDIO, NULL, 0 } },
    { 1, 1, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveDataRangePointers), FtWaveDataRangePointers,
        KSPIN_DATAFLOW_OUT, KSPIN_COMMUNICATION_SINK, &KSCATEGORY_AUDIO,
        NULL, 0 } },
#else
    // Pin 0: puente render (el audio sale a la topologia): OUT.
    { 0, 0, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveBridgeRangePtr), FtWaveBridgeRangePtr,
        KSPIN_DATAFLOW_OUT, KSPIN_COMMUNICATION_NONE, &KSCATEGORY_AUDIO, NULL, 0 } },
    // Pin 1: streaming render, 1 instancia.
    { 1, 1, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveDataRangePointers), FtWaveDataRangePointers,
        KSPIN_DATAFLOW_IN, KSPIN_COMMUNICATION_SINK, &KSCATEGORY_AUDIO,
        NULL, 0 } },
    // Pin 2: puente captura (el audio entra de la topologia): IN.
    { 0, 0, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveBridgeRangePtr), FtWaveBridgeRangePtr,
        KSPIN_DATAFLOW_IN, KSPIN_COMMUNICATION_NONE, &KSCATEGORY_AUDIO, NULL, 0 } },
    // Pin 3: streaming captura, 1 instancia.
    { 1, 1, 0, NULL,
      { 0, NULL, 0, NULL,
        SIZEOF_ARRAY(FtWaveDataRangePointers), FtWaveDataRangePointers,
        KSPIN_DATAFLOW_OUT, KSPIN_COMMUNICATION_SINK, &KSCATEGORY_AUDIO,
        NULL, 0 } },
#endif
};

// Un SUM por direccion (cada entrada de nodo solo admite una conexion).
static PCNODE_DESCRIPTOR FtWaveMiniportNodes[] =
{
    { 0, NULL, &KSNODETYPE_SUM, NULL },
    { 0, NULL, &KSNODETYPE_SUM, NULL }
};
#define KSNODE_WAVE_SUM_R 0
#define KSNODE_WAVE_SUM_C 1

// Filtro wave unico con 4 pines: puente+streaming por direccion.
// Las conexiones siguen el flujo de datos (como SysVad):
// render: host sink -> SUM -> puente; captura: puente -> SUM -> host.
static PCCONNECTION_DESCRIPTOR FtWaveMiniportConnections[] =
{
    { PCFILTER_NODE, KSPIN_WAVE_RENDER_SINK,    KSNODE_WAVE_SUM_R, 1 },
    { KSNODE_WAVE_SUM_R, 0,                     PCFILTER_NODE,     KSPIN_WAVE_RENDER_BRIDGE },
    { PCFILTER_NODE, KSPIN_WAVE_CAPTURE_BRIDGE, KSNODE_WAVE_SUM_C, 1 },
    { KSNODE_WAVE_SUM_C, 0,                     PCFILTER_NODE,     KSPIN_WAVE_CAPTURE_SOURCE },
};

static PCFILTER_DESCRIPTOR FtWaveMiniportFilterDescriptor =
{
    0, &AutomationWaveFilter,
    sizeof(PCPIN_DESCRIPTOR), SIZEOF_ARRAY(FtWaveMiniportPins), FtWaveMiniportPins,
    sizeof(PCNODE_DESCRIPTOR), SIZEOF_ARRAY(FtWaveMiniportNodes), FtWaveMiniportNodes,
    SIZEOF_ARRAY(FtWaveMiniportConnections), FtWaveMiniportConnections,
    0, NULL
};
