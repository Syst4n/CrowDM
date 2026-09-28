import {
  AppWindow,
  File,
  FileArchive,
  FileAudio,
  FileImage,
  FileText,
  FileVideo,
  MagnetStraight,
} from "@phosphor-icons/react";

export function FileIcon({ category, size = 18 }: { category: string; size?: number }) {
  const props = { size, weight: "regular" as const };
  switch (category) {
    case "videos":
      return <FileVideo {...props} />;
    case "audio":
      return <FileAudio {...props} />;
    case "images":
      return <FileImage {...props} />;
    case "documents":
      return <FileText {...props} />;
    case "archives":
      return <FileArchive {...props} />;
    case "programs":
      return <AppWindow {...props} />;
    case "torrents":
      return <MagnetStraight {...props} />;
    default:
      return <File {...props} />;
  }
}
