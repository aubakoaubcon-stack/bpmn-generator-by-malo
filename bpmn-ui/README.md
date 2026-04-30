# BPMN UI (DSL → BPMN + Editor)

This folder contains a small local UI:

- `server/`: HTTP endpoint that runs `bpmn-parser.exe` and returns BPMN XML
- `web/`: React app with embedded `bpmn-js` Modeler (Camunda-like editor)

## Run

### 1) Start the server

```powershell
cd D:\downloads\oqfuuc\bpmn-ui\server
npm install
npm run dev
```

Server: `http://localhost:5175`

### 2) Start the web app

```powershell
cd D:\downloads\oqfuuc\bpmn-ui\web
npm install
npm run dev
```

Web UI: `http://localhost:5174`

## Usage

- Paste/write DSL in the left textarea
- Click **Generate from DSL** to generate BPMN XML and load it into the editor
- Edit diagram directly
- Click **Download .bpmn** to export the current diagram
- (Optional) paste BPMN XML in the XML textarea and click **Load XML**

