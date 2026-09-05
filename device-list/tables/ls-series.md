# LS Series Display Specification

<table>
    <tr>
        <th>Model</th>
        <th>Display Type</th>
        <th>Display Area</th>
        <th>Notes</th>
    </tr>
    <tr>
        <td>LS520 SE Digital</td>
        <td>LED Digital Display</td>
        <td>Fixed segments</td>
        <td>Digital variant only</td>
    </tr>
    <tr>
        <td>LS720 SE Digital</td>
        <td>LED Digital Display</td>
        <td>Fixed segments</td>
        <td>Digital variant only</td>
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
        <td>LS520 SE Digital / LS720 SE Digital</td>
        <td>0x3633</td>
        <td>0x0006 (6)</td>
        <td>USB HID</td>
        <td>IF 0, EP OUT 0x01 (64-byte writes)</td>
        <td>Template/segment</td>
        <td>Binary packet protocol, mode byte in D1 and value bytes D2..D6</td>
        <td><a href="../../src/devices/ls_series.rs">src/devices/ls_series.rs</a></td>
        <td>Implemented mapping path exists</td>
    </tr>
</table>

# LS Series Mapping Table

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
        <td rowspan="4">D1</td>
        <td>170</td>
        <td>STATUS BAR ANIMATION</td>
    </tr>
    <tr>
        <td>19</td>
        <td>TEMPERATURE MODE ˚C</td>
    </tr>
    <tr>
        <td>35</td>
        <td>TEMPERATURE MODE ˚F</td>
    </tr>
    <tr>
        <td>76</td>
        <td>POWER MODE</td>
    </tr>
    <tr>
        <td>D2</td>
        <td>1-10</td>
        <td>STATUS BAR VALUE</td>
    </tr>
    <tr>
        <td>D3</td>
        <td>1-9</td>
        <td>NUMERIC DISPLAY || Oxx</td>
    </tr>
    <tr>
        <td>D4</td>
        <td>1-9</td>
        <td>NUMERIC DISPLAY || xOx</td>
    </tr>
    <tr>
        <td>D5</td>
        <td>1-9</td>
        <td>NUMERIC DISPLAY || xxO</td>
    </tr>
    <tr>
        <td>D6</td>
        <td>1</td>
        <td>ALARM ON</td>
    </tr>
    <tr>
        <td>...</td>
        <td>...</td>
        <td>- NOT USED -</td>
    </tr>
</table>
