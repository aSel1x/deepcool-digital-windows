# LQ Series Display Specification

<table>
    <tr>
        <th>Model</th>
        <th>Display Type</th>
        <th>Display Area</th>
        <th>Notes</th>
    </tr>
    <tr>
        <td>LQ240</td>
        <td>LED Digital Display</td>
        <td>53.9x49.15 mm</td>
        <td>Template/segment display</td>
    </tr>
    <tr>
        <td>LQ360</td>
        <td>LED Digital Display</td>
        <td>53.9x49.15 mm</td>
        <td>Template/segment display</td>
    </tr>
    <tr>
        <td>LQ360 ULTRA ARGB</td>
        <td>LED Digital Display</td>
        <td>53.9x49.15 mm</td>
        <td>Template/segment display</td>
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
        <th>Packet Model</th>
        <th>src/devices</th>
        <th>Status</th>
    </tr>
    <tr>
        <td>LQ240 / LQ360 / LQ360 ULTRA</td>
        <td>0x3633</td>
        <td>0x000D (13)</td>
        <td>USB HID</td>
        <td>IF 0, EP OUT 0x01 (64-byte writes)</td>
        <td>Template/segment</td>
        <td>Binary packet protocol with fixed header and checksum byte at D17</td>
        <td><a href="../../src/devices/lq_series.rs">src/devices/lq_series.rs</a></td>
        <td>Implemented mapping path exists</td>
    </tr>
</table>

# LQ Series / ASSASSIN IV / AK G2 Series / AK700 Mapping Table
<table>
    <tr>
        <th>DATA BYTE</th>
        <th>VALUE</th>
        <th>FUNCTION</th>
    </tr>
    <tr>
        <td>D0</td>
        <td>16</td>
        <td>REPORT ID</td>
    </tr>
    <tr>
        <td>D1</td>
        <td>104</td>
        <td rowspan="6">FIXED HEADER</td>
    </tr>
    <tr>
        <td>D2</td>
        <td>1</td>
    </tr>
    <tr>
        <td>D3</td>
        <td>8</td>
    </tr>
    <tr>
        <td>D4</td>
        <td>12</td>
    </tr>
    <tr>
        <td>D5</td>
        <td>1</td>
    </tr>
    <tr>
        <td>D6</td>
        <td>2</td>
    </tr>
    <tr>
        <td>D7</td>
        <td>0-255</td>
        <td rowspan="2">POWER CONSUMPTION <sup><code>U16</code></sup></td>
    </tr>
    <tr>
        <td>D8</td>
        <td>1-255</td>
    </tr>
    <tr>
        <td>D9</td>
        <td>0-1</td>
        <td>TEMPERATURE UNIT ˚C / ˚F</td>
    </tr>
    <tr>
        <td>D10</td>
        <td>0-255</td>
        <td rowspan="4">TEMPERATURE <sup><code>F32</code></sup></td>
    </tr>
    <tr>
        <td>D11</td>
        <td>0-255</td>
    </tr>
    <tr>
        <td>D12</td>
        <td>0-255</td>
    </tr>
    <tr>
        <td>D13</td>
        <td>1-255</td>
    </tr>
    <tr>
        <td>D14</td>
        <td>0-100</td>
        <td>UTILIZATION</td>
    </tr>
    <tr>
        <td>D15</td>
        <td>0-255</td>
        <td rowspan="2">FREQUENCY <sup><code>U16</code></sup></td>
    </tr>
    <tr>
        <td>D16</td>
        <td>1-255</td>
    </tr>
    <tr>
        <td>D17</td>
        <td>0-255</td>
        <td>D1-D16 CHECKSUM <sup><code>U8</code> REMAINDER</sup></td>
    </tr>
    <tr>
        <td>D18</td>
        <td>22</td>
        <td>TERMINATION BYTE</td>
    </tr>
    <tr>
        <td>...</td>
        <td>...</td>
        <td>- NOT USED -</td>
    </tr>
</table>
