import { Route, Routes } from "react-router-dom";
import { Toaster } from "sonner";
import { BootGate } from "./pages/BootGate";
import { ExecDetailPage } from "./pages/ExecDetail";
import { AttemptDetailPage } from "./pages/AttemptDetail";
import { HandlersPage } from "./pages/HandlersPage";
import { NewExecutionWizardPage } from "./pages/NewExecutionWizard";
import { SettingsPage } from "./pages/Settings";

export default function App() {
  return (
    <>
      <Routes>
        <Route path="/" element={<BootGate />} />
        <Route path="/new" element={<NewExecutionWizardPage />} />
        <Route path="/exec/:id" element={<ExecDetailPage />} />
        <Route path="/exec/:id/attempt/:aid" element={<AttemptDetailPage />} />
        <Route path="/handlers" element={<HandlersPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<BootGate />} />
      </Routes>
      <Toaster richColors position="bottom-right" />
    </>
  );
}
