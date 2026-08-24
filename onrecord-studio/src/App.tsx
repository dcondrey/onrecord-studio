import React, { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from './store/useAppStore';

// Safely mount your new modular dashboard layouts
import Sidebar from './components/dashboard/Sidebar';
import Header from './components/dashboard/Header';
import MainLayout from './components/dashboard/MainLayout';

export default function App() {
  const { 
    appendSyncLog, 
    incrementMetric, 
    addBytesTransferred 
  } = useAppStore();

  useEffect(() => {
    /**
     * Set up a native Tauri event listener to hook your frontend
     * view directly to the background multi-threaded Rust queue loops.
     */
    const setupTelemetryListener = async () => {
      const unlisten = await listen('ingestion-telemetry', (event: any) => {
        const telemetryEvent = event.payload;
        
        switch (telemetryEvent.type) {
          case 'pageStarted':
            appendSyncLog(`📡 Network task initiated: Requesting page ${telemetryEvent.payload.page} from ${telemetryEvent.payload.subdomain}`);
            break;
            
          case 'pageCompleted':
            incrementMetric('pagesProcessed');
            appendSyncLog(`🎉 API segment parsing finalized. Processed ${telemetryEvent.payload.recordsFound} cases on page ${telemetryEvent.payload.page}`);
            break;
            
          case 'documentStarted':
            appendSyncLog(`📂 Extracting attachment: Downloading file matrix node: ${telemetryEvent.payload.fileName} (ID: ${telemetryEvent.payload.documentId})`);
            break;
            
          case 'documentCompleted':
            incrementMetric('documentsDownloaded');
            addBytesTransferred(telemetryEvent.payload.bytesSaved);
            appendSyncLog(`💾 Disk synchronization successful: Persisted ${telemetryEvent.payload.bytesSaved} bytes into app storage.`);
            break;
            
          case 'taskFailed':
            incrementMetric('anomaliesFound');
            appendSyncLog(`⚠️ Pipeline Warning [${telemetryEvent.payload.targetId}]: Processing detour encountered -> ${telemetryEvent.payload.errorMsg}`);
            break;
            
          default:
            break;
        }
      });

      return unlisten;
    };

    const telemetryPromise = setupTelemetryListener();

    // Safely un-subscribe from Tauri event streams when the parent context unmounts
    return () => {
      telemetryPromise.then((unlistenFn) => unlistenFn());
    };
  }, [appendSyncLog, incrementMetric, addBytesTransferred]);

  return (
    <div className="w-screen h-screen flex bg-slate-950 text-slate-100 font-sans antialiased overflow-hidden select-none">
      {/* 1. COMPACT SIDEBAR NAVIGATION LAYOUT */}
      <Sidebar />

      {/* 2. OPERATIONAL DASHBOARD VIEWPORTS */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* UPPER APPLICATION STATE REFACTOR BAR */}
        <Header />

        {/* WORKBENCH DISCOVERY GRID PANEL */}
        <MainLayout />
      </div>
    </div>
  );
}
