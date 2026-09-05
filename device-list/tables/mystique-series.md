# MYSTIQUE Series Display Specification

<table>
    <tr>
        <th>Model</th>
        <th>Display Type</th>
        <th>Resolution</th>
        <th>Orientation</th>
        <th>Panel</th>
        <th>Notes</th>
    </tr>
    <tr>
        <td>MYSTIQUE 240</td>
        <td>TFT LCD (Framebuffer)</td>
        <td>480x640</td>
        <td>Portrait</td>
        <td>TFT LCD</td>
        <td>2.83 inch display</td>
    </tr>
    <tr>
        <td>MYSTIQUE 240 ARGB</td>
        <td>TFT LCD (Framebuffer)</td>
        <td>480x640</td>
        <td>Portrait</td>
        <td>TFT LCD</td>
        <td>2.83 inch display</td>
    </tr>
    <tr>
        <td>MYSTIQUE 360</td>
        <td>TFT LCD (Framebuffer)</td>
        <td>480x640</td>
        <td>Portrait</td>
        <td>TFT LCD</td>
        <td>2.83 inch display</td>
    </tr>
    <tr>
        <td>MYSTIQUE 360 ARGB</td>
        <td>TFT LCD (Framebuffer)</td>
        <td>480x640</td>
        <td>Portrait</td>
        <td>TFT LCD</td>
        <td>2.83 inch display</td>
    </tr>
</table>

## MYSTIQUE Command Reference

<table>
    <tr>
        <th>Item</th>
        <th>Value</th>
        <th>Notes</th>
    </tr>
    <tr>
        <td>VID/PID</td>
        <td>0x3633 / 0x0009</td>
        <td>MYSTIQUE 240 / 360</td>
    </tr>
    <tr>
        <td>Interface / Endpoint</td>
        <td>IF 0, EP OUT 0x01</td>
        <td>Observed bulk-style writes</td>
    </tr>
    <tr>
        <td>Known INIT</td>
        <td>0A EA 07 02 02 02 27 21</td>
        <td>Seen after cold boot before regular data writes</td>
    </tr>
    <tr>
        <td>Known data packet sample</td>
        <td>1B 00 F0 16 CC 21 01 E6 FF FF 00 00 00 00 09 00 00 02 00 02 00 01 03 40 00 00 00 FF FF FF F5 F6 F7</td>
        <td>Partial sample, full packet semantics still under reverse engineering</td>
    </tr>
</table>

## Implementation Profile

<table>
    <tr>
        <th>Series</th>
        <th>VID</th>
        <th>PID</th>
        <th>Transport</th>
        <th>Interface / Endpoint</th>
        <th>Render Model</th>
        <th>Frame Format</th>
        <th>Init / Commands</th>
        <th>src/devices</th>
        <th>Status</th>
    </tr>
    <tr>
        <td>MYSTIQUE 240 / 360</td>
        <td>0x3633</td>
        <td>0x0009 (9)</td>
        <td>USB HID / Bulk</td>
        <td>IF 0, EP OUT 0x01</td>
        <td>Framebuffer</td>
        <td>480x640 panel, packet format still under reverse engineering</td>
        <td>Known init observed: 0A EA 07 02 02 02 27 21</td>
        <td>TBD</td>
        <td>Display spec confirmed, protocol partially known</td>
    </tr>
</table>

*The repository has display specifications for these variants, but protocol mapping is still incomplete.*
