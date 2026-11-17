// Probe: does SetFileInformationByHandle(FileRenameInfo) accept a non-NULL RootDirectory?
#include <windows.h>
#include <winternl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef NTSTATUS (NTAPI *NtSetInformationFile_t)(HANDLE, PIO_STATUS_BLOCK, PVOID, ULONG, ULONG);

static int try_rename(const wchar_t *dir, const wchar_t *src, const wchar_t *dst, int use_nt) {
    HANDLE hdir = CreateFileW(dir, FILE_LIST_DIRECTORY | FILE_TRAVERSE | SYNCHRONIZE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                              NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, NULL);
    wchar_t full[MAX_PATH];
    swprintf(full, MAX_PATH, L"%s\\%s", dir, src);
    HANDLE h = CreateFileW(full, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL); CloseHandle(h);
    h = CreateFileW(full, DELETE | FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, NULL, OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT, NULL);
    size_t nlen = wcslen(dst) * 2;
    size_t len = sizeof(FILE_RENAME_INFO) + nlen;
    FILE_RENAME_INFO *info = (FILE_RENAME_INFO *)calloc(1, len);
    info->ReplaceIfExists = FALSE;
    info->RootDirectory = hdir;
    info->FileNameLength = (DWORD)nlen;
    memcpy(info->FileName, dst, nlen);
    int ok; DWORD err = 0;
    if (!use_nt) {
        ok = SetFileInformationByHandle(h, FileRenameInfo, info, (DWORD)len);
        if (!ok) err = GetLastError();
    } else {
        NtSetInformationFile_t f = (NtSetInformationFile_t)GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtSetInformationFile");
        IO_STATUS_BLOCK io;
        NTSTATUS st = f(h, &io, info, (ULONG)len, 10 /* FileRenameInformation */);
        ok = st >= 0; err = (DWORD)st;
    }
    printf("%-28s ok=%d err=0x%lx (%lu)\n", use_nt ? "NtSetInformationFile+root" : "SetFileInformationByHandle+root", ok, err, err);
    CloseHandle(h); CloseHandle(hdir); free(info);
    return ok;
}

int main(int argc, char **argv) {
    wchar_t dir[MAX_PATH];
    MultiByteToWideChar(CP_UTF8, 0, argv[1], -1, dir, MAX_PATH);
    CreateDirectoryW(dir, NULL);
    try_rename(dir, L"a1.tmp", L"b1.zim", 0);
    try_rename(dir, L"a2.tmp", L"b2.zim", 1);
    return 0;
}
